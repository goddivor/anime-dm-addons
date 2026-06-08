# anime-dm-addons

Source addons for [anime-dm](https://github.com/goddivor/anime-dm), inspired by the
Aniyomi/Tachiyomi extension model. Each addon is a Rust crate compiled to a
sandboxed **WASM** module (loaded by the host via [Extism](https://extism.org/)),
and contains **all the site-specific scraping and video-extraction logic**. The
host app only knows the shared contract and does the downloading.

## Layout

```
addons/<lang>/<source>/   # one crate = one .wasm (e.g. addons/fr/voiranime)
lib/extractors/*          # shared video-host extractors (planned)
template/                 # starting point for a new addon (planned)
```

The shared contract (`addon-api`: models + function names) lives in the host repo
`anime-dm` and is referenced by path (`../anime-dm/src-tauri/addon-api`), so both
repos must sit side by side under the same parent directory.

## An addon implements

`metadata`, `popular`, `latest`, `search`, `anime_details`, `episode_list`,
`hoster_list`, `video_list` — each takes/returns JSON. HTTP goes through Extism's
built-in client (gated by the host's allowed-hosts); HTML parsing is done in-module
with `scraper`; pages that need a real browser call the host function
`host_headless_capture`.

## Build

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
# -> target/wasm32-unknown-unknown/release/<source>.wasm
```
