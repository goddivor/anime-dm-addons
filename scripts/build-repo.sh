#!/usr/bin/env bash
# Build every addon to WASM and assemble a publishable repo/ directory:
#   repo/index.min.json        — the store index the app fetches
#   repo/<id>/addon.wasm       — the addon module
#   repo/<id>/icon.png         — the source icon (optional)
#
# Publish `repo/` (e.g. push to a `repo` branch) and point the app at the raw
# URL of repo/index.min.json.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/repo"
TARGET="wasm32-unknown-unknown"

rustup target add "$TARGET" >/dev/null 2>&1 || true
cargo build --release --target "$TARGET" --manifest-path "$ROOT/Cargo.toml"

rm -rf "$OUT"
mkdir -p "$OUT"
entries=()

while IFS= read -r meta; do
  dir="$(dirname "$meta")"
  id="$(grep -oP '"id"\s*:\s*"\K[^"]+' "$meta")"
  name="$(grep -oP '"name"\s*:\s*"\K[^"]+' "$meta")"
  lang="$(grep -oP '"lang"\s*:\s*"\K[^"]+' "$meta")"
  nsfw="$(grep -oP '"nsfw"\s*:\s*\K(true|false)' "$meta" || echo false)"
  crate="$(grep -oP '^name\s*=\s*"\K[^"]+' "$dir/Cargo.toml")"
  version="$(grep -oP '^version\s*=\s*"\K[^"]+' "$dir/Cargo.toml")"
  wasm="$ROOT/target/$TARGET/release/${crate}.wasm"

  [ -f "$wasm" ] || { echo "missing wasm for $id ($wasm)"; exit 1; }
  mkdir -p "$OUT/$id"
  cp "$wasm" "$OUT/$id/addon.wasm"

  icon_field=""
  if [ -f "$dir/icon.png" ]; then
    cp "$dir/icon.png" "$OUT/$id/icon.png"
    icon_field=", \"icon\": \"$id/icon.png\""
  fi

  entries+=("{\"id\":\"$id\",\"name\":\"$name\",\"lang\":\"$lang\",\"version\":\"$version\",\"nsfw\":$nsfw,\"wasm\":\"$id/addon.wasm\"$icon_field}")
done < <(find "$ROOT/addons" -name addon.json)

{
  printf '{"addons":['
  IFS=,; printf '%s' "${entries[*]}"; unset IFS
  printf ']}'
} > "$OUT/index.min.json"

echo "wrote $OUT/index.min.json (${#entries[@]} addon(s))"

# Repo metadata + icon (optional, shown in the app's repo list)
[ -f "$ROOT/repo.json" ] && cp "$ROOT/repo.json" "$OUT/repo.json" && echo "copied repo.json"
[ -f "$ROOT/repo-icon.png" ] && cp "$ROOT/repo-icon.png" "$OUT/repo-icon.png" && echo "copied repo-icon.png"
