#!/usr/bin/env bash
# Build Modo from source and install it for the current user (macOS or Linux).
#   macOS: Modo.app -> /Applications (or ~/Applications), `modo` CLI -> PATH
#   Linux: binary -> ~/.local/bin, desktop entry and icon -> ~/.local/share
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

link_cli() {
  local source="$1" dir=/usr/local/bin
  if [[ ! -w "$dir" ]]; then dir="$HOME/.local/bin"; mkdir -p "$dir"; fi
  ln -sf "$source" "$dir/modo"
  echo "CLI:  $dir/modo"
  case ":$PATH:" in *":$dir:"*) ;; *) echo "      Add $dir to your PATH to run \`modo\` from a terminal." ;; esac
}

case "$(uname -s)" in
  Darwin)
    scripts/package-macos.sh
    apps=/Applications
    [[ -w "$apps" ]] || { apps="$HOME/Applications"; mkdir -p "$apps"; }
    rm -rf "$apps/Modo.app"
    cp -R dist/Modo.app "$apps/"
    echo "App:  $apps/Modo.app"
    link_cli "$apps/Modo.app/Contents/Resources/bin/modo"
    ;;
  Linux)
    cargo build --locked --release
    mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications" \
      "$HOME/.local/share/icons/hicolor/512x512/apps"
    install -m 755 target/release/modo "$HOME/.local/bin/modo"
    install -m 644 assets/package/modo.png "$HOME/.local/share/icons/hicolor/512x512/apps/dev.modo.Modo.png"
    install -m 644 assets/package/dev.modo.Modo.desktop "$HOME/.local/share/applications/"
    command -v update-desktop-database >/dev/null && update-desktop-database "$HOME/.local/share/applications" || true
    echo "App:  $HOME/.local/bin/modo (also in your app launcher)"
    ;;
  *) echo "Use scripts/package-windows.ps1 on Windows." >&2; exit 1 ;;
esac
