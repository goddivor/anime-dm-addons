use std::collections::{BTreeMap, HashSet};

use addon_api::{
    Anime, AnimesPage, Episode, Hoster, Metadata, PageInput, Preference, PreferenceKind,
    SearchInput, UrlInput, Video,
};
use extism_pdk::*;
use scraper::{Html, Selector};

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const DEFAULT_BASE: &str = "https://voir-anime.to";
const PREF_BASE_URL: &str = "base_url";
const PREF_PLAYER: &str = "preferred_player";
const PREF_QUALITY: &str = "quality";
const PREF_COVER: &str = "cover_source";
const AUTO: &str = "Auto";
const COVER_VA: &str = "Voir-Anime";
const COVER_MAL: &str = "MAL";

const PLAYERS: [&str; 4] = ["LECTEUR myTV", "LECTEUR Stape", "LECTEUR VOE", "LECTEUR FHD1"];

const QUALITIES: [&str; 4] = ["Auto", "1080", "720", "480"];
const COVERS: [&str; 2] = [COVER_VA, COVER_MAL];

fn cfg(key: &str) -> Option<String> {
    config::get(key).ok().flatten().filter(|s| !s.is_empty())
}

/// The site URL, overridable from the app (Extism plugin config), else the default.
fn base_url() -> String {
    cfg(PREF_BASE_URL)
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_else(|| DEFAULT_BASE.to_string())
}

/// Fetch a URL as text through the host's HTTP capability, with a browser UA + extra headers.
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
    fetch(url, &[("Referer", &referer)])
}

/// Extract the Voir-Anime slug from an anime URL (`.../anime/<slug>/`).
fn slug_from_url(url: &str) -> Option<String> {
    let after = url.split("/anime/").nth(1)?;
    let slug = after.trim_start_matches('/').split('/').next()?.trim();
    (!slug.is_empty()).then(|| slug.to_string())
}

/// Resolve the MAL cover for a Voir-Anime URL: slug -> nijihub mal_id -> Jikan image.
/// Returns `None` (caller keeps the site poster) on any failure.
fn mal_cover(url: &str) -> Option<String> {
    let slug = slug_from_url(url)?;
    let nh = fetch(
        &format!("https://api.nijihub.com/api/v1/id-map/lookup?va_slug={slug}"),
        &[],
    )
    .ok()?;
    let nv: serde_json::Value = serde_json::from_str(&nh).ok()?;
    let mal_id = nv["data"].get(0)?.get("mal_id")?.as_i64()?;

    let jk = fetch(&format!("https://api.jikan.moe/v4/anime/{mal_id}"), &[]).ok()?;
    let jv: serde_json::Value = serde_json::from_str(&jk).ok()?;
    let images = &jv["data"]["images"];
    images["jpg"]["large_image_url"]
        .as_str()
        .or_else(|| images["jpg"]["image_url"].as_str())
        .or_else(|| images["webp"]["large_image_url"].as_str())
        .map(|s| s.to_string())
}

fn host_of(url: &str) -> String {
    url.split('/').nth(2).unwrap_or(url).to_lowercase()
}

/// Only hosters we can actually decode in `video_list` are worth returning.
fn is_supported(host: &str) -> bool {
    host.contains("vidmoly")
        || host.contains("streamtape")
        || host.contains("mail.ru")
        || host.contains("voe")
}

fn abs_url(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("//") {
        format!("https:{rest}")
    } else {
        s.to_string()
    }
}

#[plugin_fn]
pub fn metadata() -> FnResult<Json<Metadata>> {
    Ok(Json(Metadata {
        id: "fr.voiranime".into(),
        name: "Voir-Anime".into(),
        lang: "fr".into(),
        base_url: base_url(),
        version: env!("CARGO_PKG_VERSION").into(),
        nsfw: false,
    }))
}

#[plugin_fn]
pub fn preferences() -> FnResult<Json<Vec<Preference>>> {
    let mut player_opts = vec![AUTO.to_string()];
    player_opts.extend(PLAYERS.iter().map(|s| s.to_string()));
    Ok(Json(vec![
        Preference {
            key: PREF_BASE_URL.into(),
            title: "URL du site".into(),
            summary: Some("Domaine de Voir-Anime (ex : https://voir-anime.to)".into()),
            default: DEFAULT_BASE.into(),
            kind: PreferenceKind::Text,
            options: Vec::new(),
        },
        Preference {
            key: PREF_PLAYER.into(),
            title: "Lecteur préféré".into(),
            summary: Some("Lecteur essayé en premier pour le téléchargement.".into()),
            default: AUTO.into(),
            kind: PreferenceKind::Select,
            options: player_opts,
        },
        Preference {
            key: PREF_QUALITY.into(),
            title: "Qualité préférée".into(),
            summary: Some("Appliquée quand le lecteur propose plusieurs qualités (FHD1).".into()),
            default: AUTO.into(),
            kind: PreferenceKind::Select,
            options: QUALITIES.iter().map(|s| s.to_string()).collect(),
        },
        Preference {
            key: PREF_COVER.into(),
            title: "Source de l'affiche".into(),
            summary: Some(
                "« MAL » récupère l'affiche depuis MyAnimeList (via nijihub + Jikan).".into(),
            ),
            default: COVER_VA.into(),
            kind: PreferenceKind::Select,
            options: COVERS.iter().map(|s| s.to_string()).collect(),
        },
    ]))
}

#[plugin_fn]
pub fn anime_details(input: Json<UrlInput>) -> FnResult<Json<Anime>> {
    let html = get(&input.0.url)?;
    let mut anime = parse_anime(&html, &input.0.url);
    if cfg(PREF_COVER).as_deref() == Some(COVER_MAL) {
        if let Some(cover) = mal_cover(&input.0.url) {
            anime.poster_url = Some(cover);
        }
    }
    Ok(Json(anime))
}

#[plugin_fn]
pub fn episode_list(input: Json<UrlInput>) -> FnResult<Json<Vec<Episode>>> {
    let html = get(&input.0.url)?;
    Ok(Json(parse_episodes(&html)))
}

#[plugin_fn]
pub fn hoster_list(input: Json<UrlInput>) -> FnResult<Json<Vec<Hoster>>> {
    let html = get(&input.0.url)?;
    let mut hosters = parse_hosters(&html);
    if let Some(pref) = cfg(PREF_PLAYER).filter(|p| p != AUTO) {
        if let Some(i) = hosters.iter().position(|h| h.name.eq_ignore_ascii_case(&pref)) {
            let chosen = hosters.remove(i);
            hosters.insert(0, chosen);
        }
    }
    Ok(Json(hosters))
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

#[plugin_fn]
pub fn video_list(input: Json<Hoster>) -> FnResult<Json<Vec<Video>>> {
    let hoster = input.0;
    let host = host_of(&hoster.url);
    let videos = if host.contains("vidmoly") {
        vidmoly(&hoster.url)?
    } else if host.contains("streamtape") {
        streamtape(&hoster.url)?
    } else if host.contains("mail.ru") {
        mailru(&hoster.url)?
    } else if host.contains("voe") {
        voe(&hoster.url)?
    } else {
        Vec::new()
    };
    Ok(Json(videos))
}

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn vidmoly(url: &str) -> Result<Vec<Video>, Error> {
    let html = fetch(
        url,
        &[("Referer", "https://vidmoly.biz/"), ("Origin", "https://vidmoly.biz")],
    )?;
    let re = regex::Regex::new(r#"file\s*:\s*["']([^"']+\.m3u8[^"']*)["']"#).unwrap();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for c in re.captures_iter(&html) {
        let u = c[1].to_string();
        if seen.insert(u.clone()) {
            out.push(Video {
                url: u,
                quality: "auto".into(),
                headers: headers(&[
                    ("Referer", "https://vidmoly.biz/"),
                    ("Origin", "https://vidmoly.biz"),
                ]),
                ..Default::default()
            });
        }
    }
    Ok(out)
}

fn streamtape(url: &str) -> Result<Vec<Video>, Error> {
    let embed = if url.contains("/e/") {
        url.to_string()
    } else if let Some(id) = url.split('/').nth(4) {
        format!("https://streamtape.com/e/{id}")
    } else {
        url.to_string()
    };
    let html = fetch(&embed, &[("Referer", "https://voir-anime.to/")])?;
    let re = regex::Regex::new(
        r#"robotlink'\)\.innerHTML\s*=\s*'([^']*)'\s*\+\s*\(\s*'([^']*)'\s*\)((?:\.substring\(\d+\))+)"#,
    )
    .unwrap();
    let Some(caps) = re.captures(&html) else {
        return Ok(Vec::new());
    };
    let part_a = &caps[1];
    let part_b = &caps[2];
    let offset: usize = regex::Regex::new(r"substring\((\d+)\)")
        .unwrap()
        .captures_iter(&caps[3])
        .filter_map(|c| c[1].parse::<usize>().ok())
        .sum();
    let trimmed_b = part_b.get(offset..).unwrap_or("");
    let video_url = format!("https:{part_a}{trimmed_b}");
    if !video_url.contains("get_video") {
        return Ok(Vec::new());
    }
    Ok(vec![Video {
        url: video_url,
        quality: "Streamtape".into(),
        headers: headers(&[("Referer", "https://streamtape.com/")]),
        ..Default::default()
    }])
}

fn mailru(embed_url: &str) -> Result<Vec<Video>, Error> {
    let Some(id) = embed_url
        .split('?')
        .next()
        .unwrap_or(embed_url)
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
    else {
        return Ok(Vec::new());
    };
    let meta_url = format!(
        "https://my.mail.ru/+/video/meta/{id}?xemail=&ajax_call=1&func_name=&mna=&mnb=&ext=1&_=1"
    );
    let body = fetch(
        &meta_url,
        &[("Referer", embed_url), ("X-Requested-With", "XMLHttpRequest")],
    )?;

    #[derive(serde::Deserialize)]
    struct Meta {
        #[serde(default)]
        videos: Vec<MetaVideo>,
    }
    #[derive(serde::Deserialize)]
    struct MetaVideo {
        key: String,
        url: String,
    }

    let meta: Meta = serde_json::from_str(&body)?;
    let want = cfg(PREF_QUALITY)
        .filter(|q| q != AUTO)
        .and_then(|q| q.parse::<u32>().ok());
    let best = match want {
        Some(target) => meta
            .videos
            .into_iter()
            .min_by_key(|v| quality_rank(&v.key).abs_diff(target)),
        None => meta.videos.into_iter().max_by_key(|v| quality_rank(&v.key)),
    };
    let Some(best) = best else {
        return Ok(Vec::new());
    };
    let url = match best.url.strip_prefix("//") {
        Some(rest) => format!("https:{rest}"),
        None => best.url,
    };
    let mut h = headers(&[("Referer", "https://my.mail.ru/")]);
    if let Some(vk) = url.split("video_key=").nth(1) {
        h.insert(
            "Cookie".to_string(),
            format!("video_key={}", vk.split('&').next().unwrap_or(vk)),
        );
    }
    Ok(vec![Video {
        url,
        quality: best.key,
        headers: h,
        ..Default::default()
    }])
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
    let encoded = raw
        .trim()
        .trim_start_matches("[\"")
        .trim_end_matches("\"]");

    let Some(meta) = voe_decrypt(encoded) else {
        return Ok(Vec::new());
    };
    let h = headers(&[("Referer", "https://voe.sx/")]);
    let mut out = Vec::new();
    if let Some(src) = meta.get("source").and_then(|v| v.as_str()) {
        out.push(Video {
            url: src.to_string(),
            quality: "Voe HLS".into(),
            headers: h.clone(),
            ..Default::default()
        });
    }
    if let Some(mp4) = meta.get("direct_access_url").and_then(|v| v.as_str()) {
        out.push(Video {
            url: mp4.to_string(),
            quality: "Voe MP4".into(),
            headers: h,
            ..Default::default()
        });
    }
    Ok(out)
}

/// Reverse VOE's `f7` obfuscation: rot13 → strip junk patterns/underscores →
/// base64 → shift each byte by -3 → reverse → base64 → JSON.
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

fn quality_rank(key: &str) -> u32 {
    key.chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

fn parse_anime(html: &str, url: &str) -> Anime {
    let doc = Html::parse_document(html);

    let title = Selector::parse(".post-title h1, .post-title h3")
        .ok()
        .and_then(|sel| {
            doc.select(&sel)
                .next()
                .map(|e| e.text().collect::<String>().trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Animé".to_string());

    let poster_url = Selector::parse(".summary_image img").ok().and_then(|sel| {
        doc.select(&sel)
            .next()
            .and_then(|e| e.value().attr("src"))
            .map(abs_url)
    });

    Anime {
        url: url.to_string(),
        title,
        poster_url,
        ..Default::default()
    }
}

fn parse_episodes(html: &str) -> Vec<Episode> {
    let doc = Html::parse_document(html);
    let li_sel = Selector::parse("li.wp-manga-chapter").unwrap();
    let a_sel = Selector::parse("a").unwrap();
    let num_re = regex::Regex::new(r"\d+(?:\.\d+)?").unwrap();

    let mut by_num: BTreeMap<i64, Episode> = BTreeMap::new();
    for li in doc.select(&li_sel) {
        let Some(a) = li.select(&a_sel).next() else {
            continue;
        };
        let Some(href) = a.value().attr("href") else {
            continue;
        };
        let raw = a.text().collect::<String>().trim().to_string();
        let number = num_re
            .find_iter(&raw)
            .last()
            .and_then(|m| m.as_str().parse::<f32>().ok())
            .unwrap_or(0.0);
        let name = if number > 0.0 {
            format!("Épisode {}", number as i64)
        } else {
            raw.clone()
        };
        by_num.entry((number * 10.0).round() as i64).or_insert(Episode {
            url: href.to_string(),
            name,
            number,
            date_upload: None,
        });
    }
    by_num.into_values().collect()
}

fn parse_hosters(html: &str) -> Vec<Hoster> {
    let Some(start) = html.find("thisChapterSources") else {
        return Vec::new();
    };
    let Some(brace) = html[start..].find('{').map(|i| start + i) else {
        return Vec::new();
    };
    let Some(json) = balanced_object(&html[brace..]) else {
        return Vec::new();
    };
    let Ok(map) = serde_json::from_str::<BTreeMap<String, String>>(json) else {
        return Vec::new();
    };

    let src_re = regex::Regex::new(r#"src=["']([^"']+)["']"#).unwrap();
    let mut hosters = Vec::new();
    for (name, iframe_html) in map {
        if let Some(c) = src_re.captures(&iframe_html) {
            let url = abs_url(&c[1]);
            if is_supported(&host_of(&url)) {
                hosters.push(Hoster { url, name });
            }
        }
    }
    hosters
}

/// Return the smallest balanced `{ ... }` slice starting at `s[0]`.
fn balanced_object(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[..=i]);
                }
            }
            _ => {}
        }
    }
    None
}
