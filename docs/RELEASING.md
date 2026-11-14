# Releasing

1. Update `version` in `Cargo.toml` and the top entry of `CHANGELOG.md`.
2. Push the reviewed commit to `main` in `yesabhishek/modo` and wait for CI to pass on every platform.
3. Tag that commit and push the tag to the Modo remote: `git tag v0.6.0 && git push modo v0.6.0` (use `origin` if it points to `yesabhishek/modo`).
4. The Release workflow builds every platform and publishes a GitHub release with the installers, checksums, and install notes from the top changelog entry. Include all changes since the previous public release in that entry.

## Build installers locally

| Platform | Command | Output in `dist/` |
| --- | --- | --- |
| macOS | `scripts/package-macos.sh` (add `--universal` for Intel + Apple silicon) | `.dmg`, `.zip`, `Modo.app` |
| Windows | `pwsh scripts/package-windows.ps1` (needs NSIS) | `-setup.exe`, `.zip` |
| Linux (Debian-based) | `scripts/package-linux.sh` | `.deb`, `.tar.gz` |

Each script also writes `THIRD_PARTY_NOTICES.md` (all dependency licenses) into the package.

## Release signing (required)

Every release publishes `modo-<version>-SHA256SUMS.txt` and a `.sig` beside it: an Ed25519 signature over that file. The app installs an update only when the signature matches `RELEASE_KEY` in `src/brand.rs` and the package matches its line in the file, so a release published by anyone without the key (a stolen GitHub token, a compromised account) is reported but never installed. Until `RELEASE_KEY` is set, the app points users at the release page instead of installing.

One-time setup:

1. `scripts/release-key.sh` creates the key (in `~/.config/modo-release/` by default) and prints the public half.
2. Put the public key in `RELEASE_KEY` in `src/brand.rs`.
3. `gh secret set RELEASE_SIGNING_KEY --repo yesabhishek/modo < ~/.config/modo-release/signing.pem`
4. Keep an offline backup of the private key.

The Release workflow refuses to publish when the secret is missing or does not match `RELEASE_KEY`. It also records build provenance for every package; anyone can check a download with `gh attestation verify <file> --repo yesabhishek/modo`.

The workflow pins every third-party action to a commit hash (Dependabot proposes updates), builds releases without a restored cache, and pins NSIS. CI runs `cargo audit` on every change and weekly.

## Code signing

Builds are not code-signed. On macOS the app carries an ad-hoc signature (Apple silicon requires one) but is not notarized, so the first launch needs one approval; the README explains how. On Windows, SmartScreen may warn until the installer builds reputation. To sign Windows builds later, set `WINDOWS_CERTIFICATE_BASE64` and `WINDOWS_CERTIFICATE_PASSWORD` as repository secrets; the script signs automatically when they exist.

## Icons

`assets/brand/app-icon.svg` is the source. Run `scripts/make-icons.sh` on macOS to regenerate `assets/package/modo.{icns,ico,png}`.

## Publisher and updates

Publisher metadata uses **Abhishek Choudhury** and **https://www.abhishekchoudhury.com** in Cargo, About, the macOS app bundle, Windows installer, and Debian control file. Keep repository and release URLs pointing to the Modo repository.

The app's updater selects an exact platform package from the latest stable GitHub release, verifies the signed SHA256SUMS file, checks the package against it, and installs it off the UI thread. It runs `curl`, `tar`, and PowerShell by their full system paths. The automatic check runs at most once a day; Check for updates always checks. Preserve the package names and checksum artifacts produced by the scripts. Release packages must carry the same version in their file names and bundle metadata. Automatic installation needs write access to the existing installation; protected Linux installations use the system package installer. Windows installs after the running app exits. Development and capture builds report a newer release but do not replace themselves.

The updater reads `tag_name`, `html_url`, and each asset's `name`, `size`, and `browser_download_url` from the release. Download progress comes from `size`.

A running Modo compares its executable with the one it was launched from whenever its window becomes active, so any install over a running copy (the updater, a DMG, `scripts/install.sh`, a package manager) ends in a Restart to update notice. The NSIS installer refuses to replace `modo.exe` while it is in use: interactively it asks for Modo to be closed, silently it exits with an error.
