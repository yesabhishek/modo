#!/usr/bin/env bash
# Build a Linux tarball (binary, desktop entry, icon) into dist/.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
cargo build --locked --release
stem="modo-$version-linux-$(uname -m)"
staging="$(mktemp -d)/$stem"
mkdir -p "$staging" dist
install -m 755 target/release/modo "$staging/modo"
install -m 644 assets/package/modo.png "$staging/dev.modo.Modo.png"
install -m 644 assets/package/dev.modo.Modo.desktop LICENSE "$staging/"
python3 scripts/third-party-notices.py --target "$(rustc -vV | sed -n 's/^host: //p')" >/dev/null
install -m 644 THIRD_PARTY_NOTICES.md "$staging/"
tar -C "$(dirname "$staging")" -czf "dist/$stem.tar.gz" "$stem"
(cd dist && sha256sum "$stem.tar.gz" > "$stem-SHA256SUMS.txt")
echo "Built dist/$stem.tar.gz"
