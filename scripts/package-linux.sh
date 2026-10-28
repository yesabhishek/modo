#!/usr/bin/env bash
# Build Linux packages into dist/: a .deb (Debian, Ubuntu: `sudo apt install ./modo_*.deb`)
# and a portable tarball (binary, desktop entry, icon).
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
cargo build --locked --release
python3 scripts/third-party-notices.py --target "$(rustc -vV | sed -n 's/^host: //p')" >/dev/null
mkdir -p dist
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Tarball
stem="modo-$version-linux-$(uname -m)"
mkdir -p "$work/$stem"
install -m 755 target/release/modo "$work/$stem/modo"
install -m 644 assets/package/modo.png "$work/$stem/dev.modo.Modo.png"
install -m 644 assets/package/dev.modo.Modo.desktop LICENSE THIRD_PARTY_NOTICES.md "$work/$stem/"
tar -C "$work" -czf "dist/$stem.tar.gz" "$stem"

# Debian package
arch="$(dpkg --print-architecture)"
deb="$work/modo_${version}_${arch}"
mkdir -p "$deb/DEBIAN" "$deb/usr/bin" "$deb/usr/share/applications" \
  "$deb/usr/share/icons/hicolor/512x512/apps" "$deb/usr/share/doc/modo"
install -m 755 target/release/modo "$deb/usr/bin/modo"
install -m 644 assets/package/dev.modo.Modo.desktop "$deb/usr/share/applications/"
install -m 644 assets/package/modo.png "$deb/usr/share/icons/hicolor/512x512/apps/dev.modo.Modo.png"
install -m 644 LICENSE "$deb/usr/share/doc/modo/copyright"
install -m 644 THIRD_PARTY_NOTICES.md "$deb/usr/share/doc/modo/"
cat > "$deb/DEBIAN/control" <<EOF
Package: modo
Version: $version
Section: devel
Priority: optional
Architecture: $arch
Maintainer: Modo <https://github.com/yesabhishek/modo>
Installed-Size: $(du -sk "$deb/usr" | cut -f1)
Depends: libc6, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libx11-xcb1, libxcb1, libfontconfig1, libfreetype6, libvulkan1, git
Recommends: mesa-vulkan-drivers
Homepage: https://github.com/yesabhishek/modo
Description: Read, review, and edit what your agents wrote
 Modo is a fast native desktop app for reviewing the code coding agents
 write: branch diffs, a code editor, project search, and a terminal.
EOF
dpkg-deb --root-owner-group --build "$deb" "dist/modo_${version}_${arch}.deb" >/dev/null

(cd dist && sha256sum "$stem.tar.gz" "modo_${version}_${arch}.deb" > "modo-$version-linux-SHA256SUMS.txt")
echo "Built dist/modo_${version}_${arch}.deb and dist/$stem.tar.gz"
