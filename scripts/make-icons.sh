#!/usr/bin/env bash
# Regenerate Modo's app icons from assets/brand/app-icon.svg.
# Produces assets/package/modo.{icns,ico,png}. macOS only (uses iconutil).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cargo build --quiet --release --manifest-path "$root/scripts/icongen/Cargo.toml" --target-dir "$work/target"
tool="$work/target/release/icongen"
"$tool" "$root/assets/brand/app-icon.svg" "$work/Modo.iconset" iconset
iconutil -c icns "$work/Modo.iconset" -o "$root/assets/package/modo.icns"
"$tool" "$root/assets/brand/app-icon.svg" "$work/ico" ico
cp "$work/ico/icon.ico" "$root/assets/package/modo.ico"
cp "$work/Modo.iconset/icon_512x512@2x.png" "$root/assets/package/modo.png"
echo "Icons written to assets/package/"
