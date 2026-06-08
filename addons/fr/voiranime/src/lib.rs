use std::collections::{BTreeMap, HashSet};

use addon_api::{
    Anime, AnimesPage, Episode, Hoster, Metadata, PageInput, SearchInput, UrlInput, Video,
};
use extism_pdk::*;
use scraper::{Html, Selector};

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const BASE: &str = "https://voir-anime.to";

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
    fetch(url, &[("Referer", BASE)])
}

fn host_of(url: &str) -> String {
    url.split('/').nth(2).unwrap_or(url).to_lowercase()
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
        base_url: BASE.into(),
        version: env!("CARGO_PKG_VERSION").into(),
        nsfw: false,
    }))
}

#[plugin_fn]
pub fn anime_details(input: Json<UrlInput>) -> FnResult<Json<Anime>> {
    let html = get(&input.0.url)?;
    Ok(Json(parse_anime(&html, &input.0.url)))
}

#[plugin_fn]
pub fn episode_list(input: Json<UrlInput>) -> FnResult<Json<Vec<Episode>>> {
    let html = get(&input.0.url)?;
    Ok(Json(parse_episodes(&html)))
}

#[plugin_fn]
pub fn hoster_list(input: Json<UrlInput>) -> FnResult<Json<Vec<Hoster>>> {
    let html = get(&input.0.url)?;
    Ok(Json(parse_hosters(&html)))
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
    let Some(best) = meta.videos.into_iter().max_by_key(|v| quality_rank(&v.key)) else {
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
            hosters.push(Hoster {
                url: abs_url(&c[1]),
                name,
            });
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
