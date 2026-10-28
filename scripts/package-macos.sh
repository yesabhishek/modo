#!/usr/bin/env bash
# Build Modo.app, a drag-to-install DMG, and a ZIP into dist/.
#
#   scripts/package-macos.sh              # this Mac's architecture
#   scripts/package-macos.sh --universal  # Apple silicon + Intel in one app
#
# The app is ad-hoc signed (required to run on Apple silicon), not notarized.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
universal=false
[[ "${1:-}" == "--universal" ]] && universal=true

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-12.0}"

if $universal; then
  for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --locked --release --target "$target"
  done
  arch="universal"
  binary="$(mktemp -d)/modo"
  lipo -create -output "$binary" \
    target/aarch64-apple-darwin/release/modo target/x86_64-apple-darwin/release/modo
else
  cargo build --locked --release
  arch="$(uname -m | sed 's/x86_64/x64/')"
  binary="target/release/modo"
fi

out="dist"
app="$out/Modo.app"
stem="modo-$version-macos-$arch"
rm -rf "$app" "$out/$stem.dmg" "$out/$stem.zip"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/bin"

cp "$binary" "$app/Contents/MacOS/modo"
cp assets/package/modo.icns "$app/Contents/Resources/modo.icns"
cp LICENSE "$app/Contents/Resources/"
python3 scripts/third-party-notices.py --target "$(rustc -vV | sed -n 's/^host: //p')" >/dev/null
cp THIRD_PARTY_NOTICES.md "$app/Contents/Resources/"

# `modo <folder>` from a terminal hands the folder to the running app.
cat > "$app/Contents/Resources/bin/modo" <<'EOF'
#!/bin/sh
app="$(cd "$(dirname "$0")/../../.." && pwd)"
if [ $# -eq 0 ]; then exec open "$app"; fi
for target in "$@"; do
  case "$target" in /*) ;; *) target="$PWD/$target" ;; esac
  open -a "$app" "$target"
done
EOF
chmod 755 "$app/Contents/Resources/bin/modo"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Modo</string>
  <key>CFBundleDisplayName</key><string>Modo</string>
  <key>CFBundleExecutable</key><string>modo</string>
  <key>CFBundleIdentifier</key><string>dev.modo.Modo</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>modo</string>
  <key>CFBundleShortVersionString</key><string>${version%%-*}</string>
  <key>CFBundleVersion</key><string>${version%%-*}</string>
  <key>LSMinimumSystemVersion</key><string>$MACOSX_DEPLOYMENT_TARGET</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <key>CFBundleDocumentTypes</key><array>
    <dict>
      <key>CFBundleTypeName</key><string>Folder</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSItemContentTypes</key><array><string>public.folder</string></array>
    </dict>
    <dict>
      <key>CFBundleTypeName</key><string>Source code</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSHandlerRank</key><string>Alternate</string>
      <key>LSItemContentTypes</key><array>
        <string>public.source-code</string><string>public.plain-text</string><string>public.data</string>
      </array>
    </dict>
  </array>
</dict></plist>
EOF
plutil -lint "$app/Contents/Info.plist" >/dev/null

codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"

# DMG: the app next to an Applications shortcut, so installing is one drag.
staging="$(mktemp -d)"
cp -R "$app" "$staging/"
ln -s /Applications "$staging/Applications"
hdiutil create -quiet -volname "Modo" -srcfolder "$staging" -format UDZO -ov "$out/$stem.dmg"
rm -rf "$staging"
ditto -c -k --keepParent "$app" "$out/$stem.zip"


(cd "$out" && shasum -a 256 "$stem.dmg" "$stem.zip" > "$stem-SHA256SUMS.txt")
echo "Built $out/$stem.dmg"
