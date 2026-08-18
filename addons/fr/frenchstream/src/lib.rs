use std::ffi::c_char;
use std::collections::{BTreeMap, BTreeSet, HashSet};

use addon_api::{
    Anime, AnimesPage, Episode, Hoster, Metadata, PageInput, Preference, PreferenceKind,
    SearchInput, UrlInput, Video,
};
use adm_abi::{answer, answer_with, Error, Json, Result};
use scraper::{Html, Selector};

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const DEFAULT_BASE: &str = "https://french-stream.one";
// Cookie that clears the site's home-made "Verification..." anti-bot gate.
const CHAL_COOKIE: &str = "fsschal=1";
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
    fetch(url, &[("Referer", &referer), ("Cookie", CHAL_COOKIE)])
}

/// Hosts that belong to FrenchStream's own infrastructure (video CDN / player
/// wrappers). They are hotlink-protected: they only serve when the Referer is
/// the FrenchStream site and the anti-bot cookie is present.
fn is_fs_host(host: &str) -> bool {
    ["fsvid", "kakaflix", "kokoflix", "fstream", "french-stream"]
        .iter()
        .any(|k| host.contains(k))
}

/// The Referer an embed expects: the FrenchStream site for its own hosts,
/// otherwise the embed's own origin.
fn embed_referer(host: &str) -> String {
    if is_fs_host(host) {
        format!("{}/", base_url())
    } else {
        format!("https://{host}/")
    }
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

/// The scheme://host origin of a URL (the page may live on a mirror domain).
fn origin_of(url: &str) -> String {
    let mut it = url.splitn(4, '/');
    let scheme = it.next().unwrap_or("https:");
    it.next();
    let host = it.next().unwrap_or("");
    format!("{scheme}//{host}")
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let q = url.split('?').nth(1)?;
    q.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
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
        id: "fr.frenchstream".into(),
        name: "FrenchStream".into(),
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
            summary: Some("Domaine de FrenchStream (ex : https://fs03.lol)".into()),
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
    let title = first_text(&doc, "h1")
        .or_else(|| {
            attr_text(&doc, "meta[property='og:title']", "content")
                .map(|t| t.split('-').next().unwrap_or(&t).trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "FrenchStream".to_string());
    let poster_url = attr_text(
        &doc,
        ".fposter img, img.dvd-thumbnail, div.fimg img, .short-poster img, img[itemprop=image]",
        "src",
    )
    .map(|s| abs_url(&s));
    let description = first_text(&doc, "span[id^='desc-'], .fdesc");
    let genres: Vec<String> = {
        let mut out = Vec::new();
        if let Ok(sel) = Selector::parse("span.fgenre a, .flist a[href*='genre']") {
            for a in doc.select(&sel) {
                let t = a.text().collect::<String>().trim().to_string();
                if !t.is_empty() {
                    out.push(t);
                }
            }
        }
        out
    };
    Ok(Json(Anime {
        url: input.0.url.clone(),
        title,
        poster_url,
        description,
        genres,
        ..Default::default()
    }))
}

/// Find the numeric content id of the anime page (FrenchStream serves player
/// data from `ep-data.php?id=<id>`). Tries, in order: a `newsid` query param,
/// a `data-id` attribute, an inline `ep-data.php?id=` / `dle_id=` reference,
/// then the leading digits of the last path segment (`/12345-slug.html`).
fn effective_id(url: &str, html: &str, doc: &Html) -> String {
    if let Some(v) = query_param(url, "newsid") {
        return v;
    }
    if let Some(v) = attr_text(doc, "[data-id]", "data-id") {
        return v;
    }
    if let Some(c) = regex::Regex::new(r"ep-data\.php\?id=(\d+)")
        .unwrap()
        .captures(html)
    {
        return c[1].to_string();
    }
    if let Some(c) = regex::Regex::new(r#"dle_id\s*=\s*['"]?(\d+)"#)
        .unwrap()
        .captures(html)
    {
        return c[1].to_string();
    }
    let last = url.split('?').next().unwrap_or(url).rsplit('/').next().unwrap_or("");
    let before = last.split('-').next().unwrap_or("");
    if !before.is_empty() && before.chars().all(|c| c.is_ascii_digit()) {
        return before.to_string();
    }
    String::new()
}

/// Pull every player URL from a film payload: `{server: {variant: url}}`.
fn film_player_urls(players: &serde_json::Value) -> Vec<String> {
    players
        .as_object()
        .map(|m| {
            m.values()
                .filter_map(|variants| {
                    variants
                        .get("default")
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            variants
                                .as_object()
                                .and_then(|o| o.values().next())
                                .and_then(|v| v.as_str())
                        })
                        .map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Encoded player URLs of one language for an episode: `lang#url`.
fn encode_lang_players(players: Option<&serde_json::Value>, lang: &str, out: &mut Vec<String>) -> bool {
    let Some(obj) = players.and_then(|v| v.as_object()) else {
        return false;
    };
    let mut any = false;
    for v in obj.values() {
        if let Some(u) = v.as_str() {
            if !u.is_empty() {
                out.push(format!("{lang}#{u}"));
                any = true;
            }
        }
    }
    any
}

fn episode_list(input: Json<UrlInput>) -> Result<Json<Vec<Episode>>> {
    let html = get(&input.0.url)?;
    let doc = Html::parse_document(&html);
    let id = effective_id(&input.0.url, &html, &doc);
    let origin = origin_of(&input.0.url);

    let body = fetch(
        &format!("{origin}/ep-data.php?id={id}"),
        &[("Referer", &input.0.url), ("Cookie", CHAL_COOKIE)],
    )?;
    let mut data: serde_json::Value =
        serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);

    // Film: a single "players" map. Series: "vf"/"vostfr" maps keyed by episode.
    if let Some(players) = data.get("players") {
        let urls = film_player_urls(players);
        if !urls.is_empty() {
            return Ok(Json(vec![Episode {
                url: urls.join(","),
                name: "Film".into(),
                number: 1.0,
                date_upload: None,
            }]));
        }
    }

    let empty = data.get("vf").and_then(|v| v.as_object()).map(|m| m.is_empty()).unwrap_or(true)
        && data.get("vostfr").and_then(|v| v.as_object()).map(|m| m.is_empty()).unwrap_or(true);

    // Some films are only served by the legacy film API.
    if empty {
        if let Ok(film_body) = fetch(
            &format!("{origin}/engine/ajax/film_api.php?id={id}"),
            &[("Referer", &input.0.url), ("Cookie", CHAL_COOKIE)],
        ) {
            if let Ok(film) = serde_json::from_str::<serde_json::Value>(&film_body) {
                if let Some(players) = film.get("players") {
                    let urls = film_player_urls(players);
                    if !urls.is_empty() {
                        return Ok(Json(vec![Episode {
                            url: urls.join(","),
                            name: "Film".into(),
                            number: 1.0,
                            date_upload: None,
                        }]));
                    }
                }
                data = film;
            }
        }
    }

    let vf = data.get("vf").and_then(|v| v.as_object());
    let vostfr = data.get("vostfr").and_then(|v| v.as_object());
    let info = data.get("info").and_then(|v| v.as_object());

    // Union of episode numbers present in either language.
    let mut nums: BTreeSet<i64> = BTreeSet::new();
    for map in [vf, vostfr].into_iter().flatten() {
        for k in map.keys() {
            if let Ok(n) = k.parse::<i64>() {
                nums.insert(n);
            }
        }
    }
    if nums.is_empty() {
        return Ok(Json(Vec::new()));
    }

    let episodes = nums
        .into_iter()
        .map(|num| {
            let key = num.to_string();
            let mut encoded = Vec::new();
            encode_lang_players(vf.and_then(|m| m.get(&key)), "vf", &mut encoded);
            encode_lang_players(vostfr.and_then(|m| m.get(&key)), "vostfr", &mut encoded);
            let title = info
                .and_then(|m| m.get(&key))
                .and_then(|v| v.get("title"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());
            let name = match title {
                Some(t) => format!("Épisode {num} - {t}"),
                None => format!("Épisode {num}"),
            };
            Episode {
                url: encoded.join(","),
                name,
                number: num as f32,
                date_upload: None,
            }
        })
        .collect();
    Ok(Json(episodes))
}

/// Wrappers that bounce through an unbounded chain of rotating VOE-alias domains.
/// The WASM HTTP client follows redirects with a fixed cap and hard-traps on
/// "too many redirects", so these hosts are skipped entirely.
fn is_blocked_host(host: &str) -> bool {
    ["kakaflix", "kokoflix"].iter().any(|k| host.contains(k))
}

/// A readable label from the host (second-level domain), e.g. `uqload.net` -> `Uqload`.
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
    // `input.url` is the encoded player list from `episode_list`:
    // "vf#url,vostfr#url" (series) or "url,url" (film, no lang prefix).
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
        if is_blocked_host(&host) || !seen.insert(url.clone()) {
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
    if is_blocked_host(&host) {
        return Ok(Json(Vec::new()));
    }
    // A single dead/looping host must never abort resolution — return empty so
    // the app falls through to the next hoster.
    let videos = if is_dood(&host) {
        dood(&url).unwrap_or_default()
    } else {
        extract(&url).unwrap_or_default()
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
/// packed JS, then grab every `.m3u8`/`.mp4` URL. Covers Uqload, VOE, VidHide
/// (Vidzy/fsvid/premium) and Filemoon (netu) without per-domain lists.
fn extract(url: &str) -> Result<Vec<Video>, Error> {
    let host = host_of(url);
    let referer = embed_referer(&host);
    let mut html = fetch(url, &[("Referer", &referer), ("Cookie", CHAL_COOKIE)])?;
    if let Some(c) = regex::Regex::new(r#"window\.location\.href\s*=\s*['"]([^'"]+)['"]"#)
        .unwrap()
        .captures(&html)
    {
        if let Ok(h2) = fetch(&c[1].to_string(), &[("Referer", &referer), ("Cookie", CHAL_COOKIE)]) {
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

    // Raw HTML + any unpacked p.a.c.k.e.d JS. We match the packer's argument
    // tail (`}('payload',radix,count,'words'.split('|')`) directly on the HTML —
    // pre-isolating the `eval(function(p,a,c,k,e,d)...)` wrapper is unreliable
    // because its body contains nested `))` that truncate a non-greedy match.
    let mut hay = html.clone();
    let packed_re = regex::Regex::new(
        r"\}\s*\(\s*'(.*?)'\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*'(.*?)'\.split\('\|'\)",
    )
    .unwrap();
    for cap in packed_re.captures_iter(&html) {
        if let Some(u) = decode_packed(&cap[1], &cap[2], &cap[4]) {
            hay.push('\n');
            hay.push_str(&u);
        }
    }
    let vid_headers = if is_fs_host(&host) {
        headers(&[("User-Agent", UA), ("Referer", &referer), ("Cookie", CHAL_COOKIE)])
    } else {
        headers(&[("User-Agent", UA), ("Referer", &referer)])
    };
    let url_re = regex::Regex::new(r#"https?://[^\s"'\\<>]+\.(?:m3u8|mp4)[^\s"'\\<>]*"#).unwrap();
    for m in url_re.find_iter(&hay) {
        let u = m.as_str().to_string();
        if seen.insert(u.clone()) {
            out.push(Video {
                url: u,
                quality: label.clone(),
                headers: vid_headers.clone(),
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

/// Dean Edwards' p.a.c.k.e.d decoder: rebuild the source from the captured
/// payload, radix and `|`-split symbol table.
fn decode_packed(payload_raw: &str, radix_str: &str, words_str: &str) -> Option<String> {
    let payload = payload_raw.replace("\\'", "'");
    let radix: usize = radix_str.parse().ok()?;
    let words: Vec<&str> = words_str.split('|').collect();
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
