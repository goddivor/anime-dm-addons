use std::collections::BTreeMap;

use addon_api::{Anime, AnimesPage, Episode, Hoster, Metadata, PageInput, SearchInput, UrlInput};
use extism_pdk::*;
use scraper::{Html, Selector};

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const BASE: &str = "https://voir-anime.to";

/// Fetch a page as text through the host's HTTP capability.
fn get(url: &str) -> Result<String, Error> {
    let req = HttpRequest::new(url)
        .with_method("GET")
        .with_header("User-Agent", UA)
        .with_header("Referer", BASE);
    let res = http::request::<()>(&req, None)?;
    Ok(String::from_utf8_lossy(&res.body()).into_owned())
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
