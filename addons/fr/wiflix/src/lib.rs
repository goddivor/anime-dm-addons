use std::ffi::c_char;
use std::collections::{BTreeMap, HashSet};

use addon_api::{
    Anime, AnimesPage, Episode, Hoster, Metadata, PageInput, Preference, PreferenceKind,
    SearchInput, UrlInput, Video,
};
use adm_abi::{answer, answer_with, Error, Json, Result};
use scraper::{Html, Selector};

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const DEFAULT_BASE: &str = "https://flemmix.city";
const PREF_BASE_URL: &str = "base_url";
const PREF_LANG: &str = "preferred_lang";
const LANGS: [&str; 2] = ["vf", "vostfr"];

fn cfg(key: &str) -> Option<String> {
    adm_abi::config(key)
}

fn base_url() -> String {
    cfg(PREF_BASE_URL)
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_else(|| DEFAULT_BASE.to_string())
}

fn pref_lang() -> String {
    cfg(PREF_LANG).unwrap_or_else(|| "vf".to_string())
}

fn fetch(url: &str, headers: &[(&str, &str)]) -> Result<String, Error> {
    let mut all = vec![("User-Agent", UA)];
    all.extend_from_slice(headers);
    adm_abi::http_get(url, &all)
}

fn get(url: &str) -> Result<String, Error> {
    let referer = base_url();
    fetch(url, &[("Referer", &referer)])
}

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn host_of(url: &str) -> String {
    url.split('/').nth(2).unwrap_or(url).to_lowercase()
}

fn abs_url(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("//") {
        format!("https:{rest}")
    } else if s.starts_with('/') {
        format!("{}{}", base_url(), s)
    } else {
        s.to_string()
    }
}

fn attr_text(doc: &Html, sel: &str, attr: &str) -> Option<String> {
    let s = Selector::parse(sel).ok()?;
    doc.select(&s)
        .next()
        .and_then(|e| e.value().attr(attr))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn first_text(doc: &Html, sel: &str) -> Option<String> {
    let s = Selector::parse(sel).ok()?;
    doc.select(&s)
        .next()
        .map(|e| e.text().collect::<String>().trim().to_string())
        .filter(|v| !v.is_empty())
}

fn metadata() -> Result<Json<Metadata>> {
    Ok(Json(Metadata {
        id: "fr.wiflix".into(),
        name: "Wiflix".into(),
        lang: "fr".into(),
        base_url: base_url(),
        version: env!("CARGO_PKG_VERSION").into(),
        nsfw: false,
    }))
}

fn preferences() -> Result<Json<Vec<Preference>>> {
    Ok(Json(vec![
        Preference {
            key: PREF_BASE_URL.into(),
            title: "URL du site".into(),
            summary: Some("Domaine de Wiflix (ex : https://flemmix.best)".into()),
            default: DEFAULT_BASE.into(),
            kind: PreferenceKind::Text,
            options: Vec::new(),
        },
        Preference {
            key: PREF_LANG.into(),
            title: "Langue des épisodes".into(),
            summary: Some("Langue essayée en premier (VF ou VOSTFR).".into()),
            default: "vf".into(),
            kind: PreferenceKind::Select,
            options: LANGS.iter().map(|s| s.to_string()).collect(),
        },
    ]))
}

fn anime_details(input: Json<UrlInput>) -> Result<Json<Anime>> {
    let html = get(&input.0.url)?;
    let doc = Html::parse_document(&html);
    let title = first_text(&doc, "h1[itemprop=name]")
        .or_else(|| {
            attr_text(&doc, "meta[property='og:title']", "content")
                .map(|t| t.split(" »").next().unwrap_or(&t).trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Wiflix".to_string());
    let poster_url = attr_text(&doc, "#posterimg, .mov-img img, img[itemprop=image]", "src")
        .map(|s| abs_url(&s));
    Ok(Json(Anime {
        url: input.0.url.clone(),
        title,
        poster_url,
        ..Default::default()
    }))
}

/// Extract the URLs from `loadVideo('url')` onclick handlers in an element list.
fn load_urls(doc: &Html, selector: &str) -> Vec<String> {
    let re = regex::Regex::new(r#"loadVideo\(\s*['"]([^'"]+)['"]"#).unwrap();
    let Ok(sel) = Selector::parse(selector) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for a in doc.select(&sel) {
        if let Some(oc) = a.value().attr("onclick") {
            if let Some(c) = re.captures(oc) {
                let u = abs_url(&c[1]);
                if seen.insert(u.clone()) {
                    out.push(u);
                }
            }
        }
    }
    out
}

fn episode_list(input: Json<UrlInput>) -> Result<Json<Vec<Episode>>> {
    let html = get(&input.0.url)?;
    let doc = Html::parse_document(&html);

    // Series: episode blocks have a class like `ep1vf` / `ep2vostfr`.
    let class_re = regex::Regex::new(r"^ep(\d+)(vf|vostfr|vs)").unwrap();
    let div_sel = Selector::parse(".hostsblock div[class]").unwrap();
    // epNum -> lang -> urls
    let mut by_ep: BTreeMap<i64, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    for div in doc.select(&div_sel) {
        let class = div.value().attr("class").unwrap_or("");
        let Some(c) = class_re.captures(class) else {
            continue;
        };
        let ep_num: i64 = c[1].parse().unwrap_or(0);
        let lang = if class.contains("vf") { "vf" } else { "vostfr" };
        let urls = load_html_urls(&div.html());
        if urls.is_empty() {
            continue;
        }
        by_ep
            .entry(ep_num)
            .or_default()
            .entry(lang.to_string())
            .or_default()
            .extend(urls);
    }

    if !by_ep.is_empty() {
        let episodes = by_ep
            .into_iter()
            .map(|(ep_num, lang_map)| {
                let mut encoded = Vec::new();
                let mut langs = Vec::new();
                for lang in ["vf", "vostfr"] {
                    if let Some(urls) = lang_map.get(lang) {
                        if !urls.is_empty() {
                            langs.push(lang.to_uppercase());
                            for u in urls {
                                encoded.push(format!("{lang}#{u}"));
                            }
                        }
                    }
                }
                Episode {
                    url: encoded.join(","),
                    name: format!("Episode {ep_num}"),
                    number: ep_num as f32,
                    date_upload: None,
                }
            })
            .collect();
        return Ok(Json(episodes));
    }

    // Film: direct `loadVideo` anchors, no episode number, no lang prefix.
    let film_urls = load_urls(
        &doc,
        ".tabs-sel.linkstab a[onclick*=loadVideo], .linkstab a[onclick*=loadVideo], .hostsblock a[onclick*=loadVideo]",
    );
    if film_urls.is_empty() {
        return Ok(Json(Vec::new()));
    }
    Ok(Json(vec![Episode {
        url: film_urls.join(","),
        name: "(FILM) Film".into(),
        number: 1.0,
        date_upload: None,
    }]))
}

/// Parse `loadVideo` URLs from a raw HTML fragment (for a single episode block).
fn load_html_urls(fragment: &str) -> Vec<String> {
    let re = regex::Regex::new(r#"loadVideo\(\s*['"]([^'"]+)['"]"#).unwrap();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for c in re.captures_iter(fragment) {
        let u = abs_url(&c[1]);
        if seen.insert(u.clone()) {
            out.push(u);
        }
    }
    out
}

/// Hosts known to be dead / ad-only — never worth offering.
fn is_dead(host: &str) -> bool {
    const DEAD: [&str; 5] = ["upns.", "hgcloud", "waaw1", "upns.pro", "upns.live"];
    DEAD.iter().any(|k| host.contains(k))
}

/// A readable label from the host (second-level domain), e.g. `vidara.to` -> `Vidara`.
fn host_label(host: &str) -> String {
    let parts: Vec<&str> = host.trim_start_matches("www.").split('.').collect();
    let name = if parts.len() >= 2 {
        parts[parts.len() - 2]
    } else {
        host
    };
    let mut chars = name.chars();
    match chars.next() {
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
        None => name.to_string(),
    }
}

fn hoster_list(input: Json<UrlInput>) -> Result<Json<Vec<Hoster>>> {
    // `input.url` is the encoded player list produced by `episode_list`:
    // "vf#url,vf#url,vostfr#url" (series) or "url,url" (film).
    let entries: Vec<(Option<String>, String)> = input
        .0
        .url
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|item| {
            if let Some((lang, u)) = item.split_once('#') {
                if lang == "vf" || lang == "vostfr" {
                    return (Some(lang.to_string()), u.to_string());
                }
            }
            (None, item.to_string())
        })
        .collect();

    let pref = pref_lang();
    let has_lang = entries.iter().any(|(l, _)| l.is_some());
    let selected: Vec<&(Option<String>, String)> = if has_lang {
        let preferred: Vec<_> = entries.iter().filter(|(l, _)| l.as_deref() == Some(&pref)).collect();
        if preferred.is_empty() {
            entries.iter().filter(|(l, _)| l.is_some()).collect()
        } else {
            preferred
        }
    } else {
        entries.iter().collect()
    };

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (lang, url) in selected {
        let host = host_of(url);
        if is_dead(&host) || !seen.insert(url.clone()) {
            continue;
        }
        let name = match lang {
            Some(l) => format!("{} · {}", l.to_uppercase(), host_label(&host)),
            None => host_label(&host),
        };
        out.push(Hoster {
            url: url.clone(),
            name,
        });
    }
    Ok(Json(out))
}

fn video_list(input: Json<Hoster>) -> Result<Json<Vec<Video>>> {
    let url = input.0.url;
    let host = host_of(&url);
    let videos = if is_dood(&host) {
        dood(&url)?
    } else {
        extract(&url)?
    };
    Ok(Json(videos))
}

fn is_dood(host: &str) -> bool {
    ["dood", "dsvplay", "myvidplay", "playmogo", "ds2play", "d000d", "all3do"]
        .iter()
        .any(|k| host.contains(k))
}

/// DoodStream token flow (all-HTTP): the page exposes `'/pass_md5/<path>/<token>'`,
/// a GET on it returns the base URL, then `base + nonce + ?token=&expiry=` is the mp4.
fn dood(url: &str) -> Result<Vec<Video>, Error> {
    let host = host_of(url);
    let referer_host = format!("https://{host}/");
    let html = fetch(url, &[("Referer", &referer_host)])?;
    let Some(md5) = html
        .split("'/pass_md5/")
        .nth(1)
        .and_then(|s| s.split('\'').next())
    else {
        return Ok(Vec::new());
    };
    let token = md5.rsplit('/').next().unwrap_or("");
    let base = fetch(&format!("https://{host}/pass_md5/{md5}"), &[("Referer", url)])?;
    let base = base.trim();
    if !base.starts_with("http") {
        return Ok(Vec::new());
    }
    let video_url = format!(
        "{base}{}?token={token}&expiry=9999999999999",
        pseudo_random(token)
    );
    Ok(vec![Video {
        url: video_url,
        quality: format!("Doodstream ({})", host_label(&host)),
        headers: headers(&[("User-Agent", UA), ("Referer", &referer_host)]),
        ..Default::default()
    }])
}

/// A deterministic 10-char alphanumeric nonce (Dood only needs *some* padding;
/// WASM has no RNG, so we derive it from the token).
fn pseudo_random(seed: &str) -> String {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut h: u64 = 1469598103934665603;
    for b in seed.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    (0..10)
        .map(|i| {
            h = h
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407 + i as u64);
            ALPHA[(h >> 33) as usize % ALPHA.len()] as char
        })
        .collect()
}

// ---------------- Universal extractor ----------------

/// Host-agnostic: fetch the embed, try VOE's encrypted JSON, then unpack any
/// packed JS, then grab every `.m3u8`/`.mp4` URL. Covers most embed players
/// (Uqload, Vidmoly, Lulu, StreamWish, Filemoon, Vido, VOE…) without per-domain
/// lists — important since these hosts rotate domains constantly.
fn extract(url: &str) -> Result<Vec<Video>, Error> {
    let host = host_of(url);
    let referer = format!("https://{host}/");
    let mut html = fetch(url, &[("Referer", &referer)])?;
    if let Some(c) = regex::Regex::new(r#"window\.location\.href\s*=\s*['"]([^'"]+)['"]"#)
        .unwrap()
        .captures(&html)
    {
        if let Ok(h2) = fetch(&c[1].to_string(), &[]) {
            html = h2;
        }
    }

    let label = host_label(&host);
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    // VOE-style: an encrypted JSON <script> payload.
    if let Some(meta) = voe_from_html(&html) {
        let vh = headers(&[("Referer", "https://voe.sx/")]);
        for key in ["source", "direct_access_url"] {
            if let Some(u) = meta.get(key).and_then(|v| v.as_str()) {
                if seen.insert(u.to_string()) {
                    out.push(Video {
                        url: u.to_string(),
                        quality: label.clone(),
                        headers: vh.clone(),
                        ..Default::default()
                    });
                }
            }
        }
        if !out.is_empty() {
            return Ok(out);
        }
    }

    // Raw HTML + any unpacked packed JS.
    let mut hay = html.clone();
    let packed_re = regex::Regex::new(r"eval\(function\(p,a,c,k,e,d\).*?\)\)").unwrap();
    for m in packed_re.find_iter(&html) {
        if let Some(u) = unpack(m.as_str()) {
            hay.push('\n');
            hay.push_str(&u);
        }
    }
    let url_re = regex::Regex::new(r#"https?://[^\s"'\\<>]+\.(?:m3u8|mp4)[^\s"'\\<>]*"#).unwrap();
    for m in url_re.find_iter(&hay) {
        let u = m.as_str().to_string();
        if seen.insert(u.clone()) {
            out.push(Video {
                url: u,
                quality: label.clone(),
                headers: headers(&[("Referer", &referer)]),
                ..Default::default()
            });
        }
    }
    Ok(out)
}

fn voe_from_html(html: &str) -> Option<serde_json::Value> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse(r#"script[type="application/json"]"#).ok()?;
    let raw = doc.select(&sel).next().map(|e| e.text().collect::<String>())?;
    let encoded = raw.trim().trim_start_matches("[\"").trim_end_matches("\"]");
    voe_decrypt(encoded)
}

fn voe_decrypt(input: &str) -> Option<serde_json::Value> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let rot13: String = input
        .chars()
        .map(|c| match c {
            'A'..='Z' => (((c as u8 - b'A' + 13) % 26) + b'A') as char,
            'a'..='z' => (((c as u8 - b'a' + 13) % 26) + b'a') as char,
            _ => c,
        })
        .collect();
    let mut cleaned = rot13;
    for p in ["@$", "^^", "~@", "%?", "*~", "!!", "#&"] {
        cleaned = cleaned.replace(p, "_");
    }
    cleaned = cleaned.replace('_', "");
    let step1 = b64.decode(cleaned.as_bytes()).ok()?;
    let shifted: Vec<u8> = step1.iter().map(|b| b.wrapping_sub(3)).collect();
    let reversed: Vec<u8> = shifted.into_iter().rev().collect();
    let step2 = b64.decode(&reversed).ok()?;
    serde_json::from_slice(&step2).ok()
}

/// Dean Edwards' p.a.c.k.e.d unpacker.
fn unpack(packed: &str) -> Option<String> {
    let re = regex::Regex::new(r"\}\s*\(\s*'(.*?)'\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*'(.*?)'\.split\('\|'\)")
        .unwrap();
    let cap = re.captures(packed)?;
    let payload = cap[1].replace("\\'", "'");
    let radix: usize = cap[2].parse().ok()?;
    let words: Vec<&str> = cap[4].split('|').collect();
    let token_re = regex::Regex::new(r"\b\w+\b").unwrap();
    let result = token_re.replace_all(&payload, |c: &regex::Captures| {
        let tok = &c[0];
        match decode_base(tok, radix) {
            Some(i) if i < words.len() && !words[i].is_empty() => words[i].to_string(),
            _ => tok.to_string(),
        }
    });
    Some(result.into_owned())
}

fn decode_base(s: &str, radix: usize) -> Option<usize> {
    let mut n = 0usize;
    for ch in s.chars() {
        let d = match ch {
            '0'..='9' => ch as usize - '0' as usize,
            'a'..='z' => ch as usize - 'a' as usize + 10,
            'A'..='Z' => ch as usize - 'A' as usize + 36,
            _ => return None,
        };
        if d >= radix {
            return None;
        }
        n = n * radix + d;
    }
    Some(n)
}

fn popular(_input: Json<PageInput>) -> Result<Json<AnimesPage>> {
    Ok(Json(AnimesPage::default()))
}

fn latest(_input: Json<PageInput>) -> Result<Json<AnimesPage>> {
    Ok(Json(AnimesPage::default()))
}

fn search(_input: Json<SearchInput>) -> Result<Json<AnimesPage>> {
    Ok(Json(AnimesPage::default()))
}

// --- C entry points ---------------------------------------------------------

#[no_mangle]
pub extern "C" fn adm_metadata() -> *mut c_char {
    answer(metadata)
}

#[no_mangle]
pub extern "C" fn adm_preferences() -> *mut c_char {
    answer(preferences)
}

/// # Safety
/// The host passes a NUL-terminated JSON argument.
#[no_mangle]
pub unsafe extern "C" fn adm_anime_details(input: *const c_char) -> *mut c_char {
    answer_with(input, anime_details)
}

/// # Safety
/// The host passes a NUL-terminated JSON argument.
#[no_mangle]
pub unsafe extern "C" fn adm_episode_list(input: *const c_char) -> *mut c_char {
    answer_with(input, episode_list)
}

/// # Safety
/// The host passes a NUL-terminated JSON argument.
#[no_mangle]
pub unsafe extern "C" fn adm_hoster_list(input: *const c_char) -> *mut c_char {
    answer_with(input, hoster_list)
}

/// # Safety
/// The host passes a NUL-terminated JSON argument.
#[no_mangle]
pub unsafe extern "C" fn adm_video_list(input: *const c_char) -> *mut c_char {
    answer_with(input, video_list)
}

/// # Safety
/// The host passes a NUL-terminated JSON argument.
#[no_mangle]
pub unsafe extern "C" fn adm_popular(input: *const c_char) -> *mut c_char {
    answer_with(input, popular)
}

/// # Safety
/// The host passes a NUL-terminated JSON argument.
#[no_mangle]
pub unsafe extern "C" fn adm_latest(input: *const c_char) -> *mut c_char {
    answer_with(input, latest)
}

/// # Safety
/// The host passes a NUL-terminated JSON argument.
#[no_mangle]
pub unsafe extern "C" fn adm_search(input: *const c_char) -> *mut c_char {
    answer_with(input, search)
}
