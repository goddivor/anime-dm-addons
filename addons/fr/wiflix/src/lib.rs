use std::collections::{BTreeMap, HashSet};

use addon_api::{
    Anime, AnimesPage, Episode, Hoster, Metadata, PageInput, Preference, PreferenceKind,
    SearchInput, UrlInput, Video,
};
use extism_pdk::*;
use scraper::{Html, Selector};

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const DEFAULT_BASE: &str = "https://fs18.lol";
const PREF_BASE_URL: &str = "base_url";
const PREF_LANG: &str = "preferred_lang";
const LANGS: [&str; 2] = ["vf", "vostfr"];

fn cfg(key: &str) -> Option<String> {
    config::get(key).ok().flatten().filter(|s| !s.is_empty())
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
    let mut req = HttpRequest::new(url)
        .with_method("GET")
        .with_header("User-Agent", UA);
    for (k, v) in headers {
        req = req.with_header(*k, *v);
    }
    let res = http::request::<()>(&req, None)?;
    Ok(String::from_utf8_lossy(&res.body()).into_owned())
}

fn get(url: &str) -> Result<String, Error> {
    let referer = base_url();
    // The site gates content behind a JS challenge that just sets this cookie.
    fetch(url, &[("Referer", &referer), ("Cookie", "fsschal=1")])
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

#[plugin_fn]
pub fn metadata() -> FnResult<Json<Metadata>> {
    Ok(Json(Metadata {
        id: "fr.wiflix".into(),
        name: "Wiflix".into(),
        lang: "fr".into(),
        base_url: base_url(),
        version: env!("CARGO_PKG_VERSION").into(),
        nsfw: false,
    }))
}

#[plugin_fn]
pub fn preferences() -> FnResult<Json<Vec<Preference>>> {
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

#[plugin_fn]
pub fn anime_details(input: Json<UrlInput>) -> FnResult<Json<Anime>> {
    let html = get(&input.0.url)?;
    let doc = Html::parse_document(&html);
    let title = attr_text(&doc, "meta[property='og:title']", "content")
        .or_else(|| first_text(&doc, "h1[itemprop=name], div.mov-title h1, h1"))
        .unwrap_or_else(|| "Wiflix".to_string());
    let poster_url = attr_text(&doc, "meta[property='og:image']", "content")
        .or_else(|| attr_text(&doc, "div.fposter img, div.mov-img img, .fpic img", "src"))
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

#[plugin_fn]
pub fn episode_list(input: Json<UrlInput>) -> FnResult<Json<Vec<Episode>>> {
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

/// Only hosters we can actually decode in `video_list` are worth returning.
fn is_supported(host: &str) -> bool {
    const KNOWN: [&str; 14] = [
        "uqload",
        "voe.sx",
        "luluvdo",
        "lulustream",
        "filelions",
        "minochinos",
        "filemoon",
        "streamwish",
        "vido.lol",
        "vudeo",
        "upstream",
        "up4fun",
        "streamvid",
        "streamdav",
    ];
    KNOWN.iter().any(|k| host.contains(k)) || host.contains("voe")
}

fn host_label(host: &str) -> &'static str {
    if host.contains("uqload") {
        "Uqload"
    } else if host.contains("voe") {
        "VOE"
    } else if host.contains("lulu") {
        "Lulu"
    } else if host.contains("filelions") || host.contains("filemoon") || host.contains("minochinos")
    {
        "Filemoon"
    } else if host.contains("streamwish") {
        "StreamWish"
    } else if host.contains("vido") {
        "Vido"
    } else if host.contains("vudeo") {
        "Vudeo"
    } else if host.contains("upstream") || host.contains("up4fun") {
        "Upstream"
    } else if host.contains("streamvid") {
        "StreamVid"
    } else if host.contains("streamdav") {
        "StreamDav"
    } else {
        "Lecteur"
    }
}

#[plugin_fn]
pub fn hoster_list(input: Json<UrlInput>) -> FnResult<Json<Vec<Hoster>>> {
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
        if !is_supported(&host) || !seen.insert(url.clone()) {
            continue;
        }
        let name = match lang {
            Some(l) => format!("{} · {}", l.to_uppercase(), host_label(&host)),
            None => host_label(&host).to_string(),
        };
        out.push(Hoster {
            url: url.clone(),
            name,
        });
    }
    Ok(Json(out))
}

#[plugin_fn]
pub fn video_list(input: Json<Hoster>) -> FnResult<Json<Vec<Video>>> {
    let hoster = input.0;
    let host = host_of(&hoster.url);
    let videos = if host.contains("uqload") {
        uqload(&hoster.url)?
    } else if host.contains("voe") {
        voe(&hoster.url)?
    } else {
        generic(&hoster.url)?
    };
    Ok(Json(videos))
}

// ---------------- Extractors ----------------

fn uqload(url: &str) -> Result<Vec<Video>, Error> {
    let host = host_of(url);
    let html = fetch(url, &[("Referer", &format!("https://{host}/"))])?;
    let re = regex::Regex::new(r#"sources\s*:\s*\[\s*["']([^"']+\.mp4[^"']*)["']"#).unwrap();
    let mut out = Vec::new();
    if let Some(c) = re.captures(&html) {
        out.push(Video {
            url: c[1].to_string(),
            quality: "Uqload".into(),
            headers: headers(&[("Referer", &format!("https://{host}/"))]),
            ..Default::default()
        });
    }
    Ok(out)
}

fn voe(url: &str) -> Result<Vec<Video>, Error> {
    let mut html = fetch(url, &[])?;
    if let Some(c) = regex::Regex::new(r#"window\.location\.href\s*=\s*'([^']+)'"#)
        .unwrap()
        .captures(&html)
    {
        html = fetch(&c[1].to_string(), &[])?;
    }
    let doc = Html::parse_document(&html);
    let sel = Selector::parse(r#"script[type="application/json"]"#).unwrap();
    let Some(raw) = doc.select(&sel).next().map(|e| e.text().collect::<String>()) else {
        return Ok(Vec::new());
    };
    let encoded = raw.trim().trim_start_matches("[\"").trim_end_matches("\"]");
    let Some(meta) = voe_decrypt(encoded) else {
        return Ok(Vec::new());
    };
    let h = headers(&[("Referer", "https://voe.sx/")]);
    let mut out = Vec::new();
    if let Some(src) = meta.get("source").and_then(|v| v.as_str()) {
        out.push(Video {
            url: src.to_string(),
            quality: "VOE HLS".into(),
            headers: h.clone(),
            ..Default::default()
        });
    }
    if let Some(mp4) = meta.get("direct_access_url").and_then(|v| v.as_str()) {
        out.push(Video {
            url: mp4.to_string(),
            quality: "VOE MP4".into(),
            headers: h,
            ..Default::default()
        });
    }
    Ok(out)
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

/// Generic extractor: fetch the embed, unpack any packed JS, then grab the
/// first `.m3u8`/`.mp4` URL. Covers the StreamWish/Filemoon/Lulu/Vido family.
fn generic(url: &str) -> Result<Vec<Video>, Error> {
    let host = host_of(url);
    let referer = format!("https://{host}/");
    let html = fetch(url, &[("Referer", &referer)])?;

    let mut haystack = html.clone();
    let packed_re = regex::Regex::new(r"eval\(function\(p,a,c,k,e,d\).*?\)\)").unwrap();
    for m in packed_re.find_iter(&html) {
        if let Some(unpacked) = unpack(m.as_str()) {
            haystack.push('\n');
            haystack.push_str(&unpacked);
        }
    }

    let url_re =
        regex::Regex::new(r#"https?://[^\s"'\\<>]+\.(?:m3u8|mp4)[^\s"'\\<>]*"#).unwrap();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for m in url_re.find_iter(&haystack) {
        let u = m.as_str().to_string();
        if seen.insert(u.clone()) {
            out.push(Video {
                url: u,
                quality: host_label(&host).into(),
                headers: headers(&[("Referer", &referer)]),
                ..Default::default()
            });
        }
    }
    Ok(out)
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

#[plugin_fn]
pub fn popular(_input: Json<PageInput>) -> FnResult<Json<AnimesPage>> {
    Ok(Json(AnimesPage::default()))
}

#[plugin_fn]
pub fn latest(_input: Json<PageInput>) -> FnResult<Json<AnimesPage>> {
    Ok(Json(AnimesPage::default()))
}

#[plugin_fn]
pub fn search(_input: Json<SearchInput>) -> FnResult<Json<AnimesPage>> {
    Ok(Json(AnimesPage::default()))
}
