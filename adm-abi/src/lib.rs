//! The C ABI every source addon exposes to the anime-dm host.
//!
//! An addon is a native dynamic library. The host loads it, hands it a table of
//! services, pushes the user settings, then calls the exported entry points with
//! JSON in and JSON out. Every string an addon returns belongs to the addon and
//! is released through [`adm_free`], so the two sides never share an allocator.

use std::collections::BTreeMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{OnceLock, RwLock};

pub use anyhow::{anyhow, Error, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Bumped whenever the shape below changes; the host refuses a mismatch.
pub const ABI_VERSION: u32 = 1;

/// Services the host lends to an addon. Kept to the strict minimum: an addon
/// never opens a socket itself, so every request goes through the host client
/// and inherits its user agent, its timeouts and its logging.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AdmHost {
    /// `size_of::<AdmHost>()`, so the host may grow the table later on.
    pub size: u32,
    /// Opaque host state, handed back with every call.
    pub ctx: *mut c_void,
    /// GET `url` with the headers of a JSON object, returning the body.
    pub http_get: Option<extern "C" fn(*mut c_void, *const c_char, *const c_char) -> *mut c_char>,
    /// Releases a string the host returned.
    pub free_string: Option<extern "C" fn(*mut c_void, *mut c_char)>,
    /// Writes a diagnostic line to the host log.
    pub log: Option<extern "C" fn(*mut c_void, *const c_char)>,
}

/// The table is filled once at load time and only read afterwards.
unsafe impl Send for AdmHost {}
unsafe impl Sync for AdmHost {}

static HOST: OnceLock<AdmHost> = OnceLock::new();

fn config_store() -> &'static RwLock<BTreeMap<String, String>> {
    static CONFIG: OnceLock<RwLock<BTreeMap<String, String>>> = OnceLock::new();
    CONFIG.get_or_init(|| RwLock::new(BTreeMap::new()))
}

// --- entry points shared by every addon -------------------------------------

/// Reports the ABI the addon was built against.
#[no_mangle]
pub extern "C" fn adm_abi_version() -> u32 {
    ABI_VERSION
}

/// Receives the host service table. Called once, right after loading.
///
/// # Safety
/// `host` points to a table the host keeps alive for the whole session.
#[no_mangle]
pub unsafe extern "C" fn adm_init(host: *const AdmHost) {
    if host.is_null() {
        return;
    }
    let _ = HOST.set(*host);
}

/// Receives the user settings as a JSON object of strings.
///
/// # Safety
/// `json` is a NUL-terminated string owned by the host.
#[no_mangle]
pub unsafe extern "C" fn adm_set_config(json: *const c_char) {
    let Some(text) = borrow(json) else {
        return;
    };
    let parsed: BTreeMap<String, String> = serde_json::from_str(text).unwrap_or_default();
    if let Ok(mut store) = config_store().write() {
        *store = parsed;
    }
}

/// Releases a string returned by one of the addon entry points.
///
/// # Safety
/// `text` comes from this library and is released only once.
#[no_mangle]
pub unsafe extern "C" fn adm_free(text: *mut c_char) {
    if !text.is_null() {
        drop(CString::from_raw(text));
    }
}

// --- helpers for the addons -------------------------------------------------

/// Reads a setting, treating an empty value as absent.
pub fn config(key: &str) -> Option<String> {
    config_store()
        .read()
        .ok()?
        .get(key)
        .filter(|s| !s.is_empty())
        .cloned()
}

/// Writes a diagnostic line to the host log.
pub fn log(message: &str) {
    let Some(host) = HOST.get() else {
        return;
    };
    let Some(sink) = host.log else {
        return;
    };
    if let Ok(text) = CString::new(message) {
        sink(host.ctx, text.as_ptr());
    }
}

/// Fetches a URL as text through the host client.
pub fn http_get(url: &str, headers: &[(&str, &str)]) -> Result<String> {
    let host = HOST.get().ok_or_else(|| anyhow!("host not initialised"))?;
    let get = host.http_get.ok_or_else(|| anyhow!("host offers no http_get"))?;

    let map: BTreeMap<&str, &str> = headers.iter().copied().collect();
    let url_c = CString::new(url)?;
    let headers_c = CString::new(serde_json::to_string(&map)?)?;

    let raw = get(host.ctx, url_c.as_ptr(), headers_c.as_ptr());
    if raw.is_null() {
        return Err(anyhow!("request failed: {url}"));
    }

    let body = unsafe { CStr::from_ptr(raw) }.to_string_lossy().into_owned();
    if let Some(free) = host.free_string {
        free(host.ctx, raw);
    }
    Ok(body)
}

// --- plumbing for the entry points ------------------------------------------

/// Borrows a NUL-terminated string the host owns.
///
/// # Safety
/// `text` is either null or a valid NUL-terminated string.
unsafe fn borrow<'a>(text: *const c_char) -> Option<&'a str> {
    if text.is_null() {
        return None;
    }
    CStr::from_ptr(text).to_str().ok()
}

/// Serialises an answer into the envelope the host expects and hands over
/// ownership of the buffer.
fn envelope<T: Serialize>(result: Result<T>) -> *mut c_char {
    let json = match result {
        Ok(value) => serde_json::json!({ "ok": value }),
        Err(error) => serde_json::json!({ "error": error.to_string() }),
    };
    let text = serde_json::to_string(&json).unwrap_or_else(|_| {
        r#"{"error":"answer could not be serialised"}"#.to_string()
    });
    CString::new(text)
        .unwrap_or_else(|_| CString::new(r#"{"error":"answer holds a null byte"}"#).unwrap())
        .into_raw()
}

/// Runs an entry point that takes no argument, turning a panic into an error.
pub fn answer<O, F>(body: F) -> *mut c_char
where
    O: Serialize,
    F: FnOnce() -> Result<O>,
{
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(result) => envelope(result),
        Err(_) => envelope::<()>(Err(anyhow!("the addon panicked"))),
    }
}

/// Runs an entry point that takes a JSON argument.
///
/// # Safety
/// `input` is a NUL-terminated string owned by the host.
pub unsafe fn answer_with<I, O, F>(input: *const c_char, body: F) -> *mut c_char
where
    I: DeserializeOwned,
    O: Serialize,
    F: FnOnce(I) -> Result<O>,
{
    let Some(text) = borrow(input) else {
        return envelope::<()>(Err(anyhow!("missing argument")));
    };
    let parsed = match serde_json::from_str::<I>(text) {
        Ok(value) => value,
        Err(error) => return envelope::<()>(Err(anyhow!("invalid argument: {error}"))),
    };
    answer(move || body(parsed))
}
