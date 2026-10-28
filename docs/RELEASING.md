# Releasing

1. Update `version` in `Cargo.toml` and the top entry of `CHANGELOG.md`.
2. Tag and push: `git tag v0.2.0 && git push origin v0.2.0`.
3. The Release workflow builds every platform and publishes a GitHub release with the installers, checksums, and install notes.

## Build installers locally

| Platform | Command | Output in `dist/` |
| --- | --- | --- |
| macOS | `scripts/package-macos.sh` (add `--universal` for Intel + Apple silicon) | `.dmg`, `.zip`, `Modo.app` |
| Windows | `pwsh scripts/package-windows.ps1` (needs NSIS) | `-setup.exe`, `.zip` |
| Linux (Debian-based) | `scripts/package-linux.sh` | `.deb`, `.tar.gz` |

Each script also writes `THIRD_PARTY_NOTICES.md` (all dependency licenses) into the package.

## Signing

Builds ship unsigned. On macOS the app carries an ad-hoc signature (Apple silicon requires one) but is not notarized, so the first launch needs one approval; the README explains how. On Windows, SmartScreen may warn until the installer builds reputation. To sign Windows builds later, set `WINDOWS_CERTIFICATE_BASE64` and `WINDOWS_CERTIFICATE_PASSWORD` as repository secrets; the script signs automatically when they exist.

## Icons

`assets/brand/app-icon.svg` is the source. Run `scripts/make-icons.sh` on macOS to regenerate `assets/package/modo.{icns,ico,png}`.
