#!/usr/bin/env bash
# Create the Ed25519 key that signs every release's SHA256SUMS file.
#
#   scripts/release-key.sh [path/to/signing.pem]
#
# Run once. Then:
#   1. Put the printed public key in RELEASE_KEY in src/brand.rs.
#   2. Store the private key as the RELEASE_SIGNING_KEY repository secret:
#        gh secret set RELEASE_SIGNING_KEY --repo yesabhishek/modo < path/to/signing.pem
#   3. Keep an offline backup of the private key. Losing it means shipping a
#      release with a new public key that users install by hand.
set -euo pipefail

key="${1:-$HOME/.config/modo-release/signing.pem}"
openssl=openssl
# macOS ships LibreSSL, which cannot make Ed25519 keys; prefer Homebrew's OpenSSL.
for candidate in /opt/homebrew/bin/openssl /usr/local/opt/openssl/bin/openssl; do
  [[ -x "$candidate" ]] && openssl="$candidate" && break
done

if [[ -e "$key" ]]; then
  echo "$key already exists; refusing to replace a signing key." >&2
  exit 1
fi
mkdir -p "$(dirname "$key")"
chmod 700 "$(dirname "$key")"
(umask 077 && "$openssl" genpkey -algorithm ed25519 -out "$key")

public="$("$openssl" pkey -in "$key" -pubout -outform DER | tail -c 32 | od -An -tx1 | tr -d ' \n')"
echo "Private key: $key"
echo "Public key:  $public"
echo
echo "Set RELEASE_KEY in src/brand.rs to the public key, then add the secret:"
echo "  gh secret set RELEASE_SIGNING_KEY --repo yesabhishek/modo < \"$key\""
