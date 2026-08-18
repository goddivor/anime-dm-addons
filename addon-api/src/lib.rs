use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Plugin exports the host calls (JSON in / JSON out), in call order.
pub mod exports {
    pub const METADATA: &str = "metadata";
    pub const POPULAR: &str = "popular";
    pub const LATEST: &str = "latest";
    pub const SEARCH: &str = "search";
    pub const ANIME_DETAILS: &str = "anime_details";
    pub const EPISODE_LIST: &str = "episode_list";
    pub const HOSTER_LIST: &str = "hoster_list";
    pub const VIDEO_LIST: &str = "video_list";
    pub const PREFERENCES: &str = "preferences";
}

/// Host functions the plugin imports (capabilities the host provides).
pub mod host_fns {
    pub const HEADLESS_CAPTURE: &str = "host_headless_capture";
    pub const LOG: &str = "host_log";
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub id: String,
    pub name: String,
    pub lang: String,
    pub base_url: String,
    pub version: String,
    #[serde(default)]
    pub nsfw: bool,
}

/// A configurable setting an addon declares; the host renders it as a settings
/// field, stores the chosen value, and feeds it back via Extism plugin config.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Preference {
    pub key: String,
    pub title: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub default: String,
    #[serde(rename = "type")]
    pub kind: PreferenceKind,
    /// For `Select`: the allowed values (label == value).
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PreferenceKind {
    Text,
    Select,
    Bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Anime {
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub poster_url: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Episode {
    pub url: String,
    pub name: String,
    pub number: f32,
    #[serde(default)]
    pub date_upload: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct AnimesPage {
    pub animes: Vec<Anime>,
    pub has_next_page: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Hoster {
    pub url: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Track {
    pub url: String,
    pub lang: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Video {
    pub url: String,
    #[serde(default)]
    pub quality: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub subtitles: Vec<Track>,
    #[serde(default)]
    pub audio: Vec<Track>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PageInput {
    pub page: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SearchInput {
    pub query: String,
    pub page: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UrlInput {
    pub url: String,
}

/// Host function `host_headless_capture` payload: load a JS-rendered page in a
/// real browser and return the captured media source (URL + required headers).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HeadlessRequest {
    pub url: String,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct HeadlessResponse {
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}
