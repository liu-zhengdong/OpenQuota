#!/usr/bin/env bash
# Import a fixed self-signed code signing certificate into a temporary keychain and
# export APPLE_SIGNING_IDENTITY for the Tauri build.
#
# Ad-hoc signatures pin the app's designated requirement to its cdhash, which changes
# with every build, so macOS Keychain forgets "Always Allow" after each update. Signing
# with the same certificate every time makes the requirement
# `identifier ... and certificate leaf = H"..."`, which stays stable across releases.
# This is not Developer ID signing: Gatekeeper still treats the app as unidentified.
#
# Required env: MACOS_SELFSIGN_CERTIFICATE (base64 .p12), MACOS_SELFSIGN_PASSWORD.
set -euo pipefail

: "${MACOS_SELFSIGN_CERTIFICATE:?MACOS_SELFSIGN_CERTIFICATE is required}"
: "${MACOS_SELFSIGN_PASSWORD:?MACOS_SELFSIGN_PASSWORD is required}"

keychain="$RUNNER_TEMP/openquota-selfsign.keychain-db"
keychain_password="$(openssl rand -hex 16)"
p12="$RUNNER_TEMP/openquota-selfsign.p12"

printf '%s' "$MACOS_SELFSIGN_CERTIFICATE" | base64 --decode > "$p12"
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$p12" -k "$keychain" -P "$MACOS_SELFSIGN_PASSWORD" -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple: -k "$keychain_password" "$keychain" >/dev/null
rm -f "$p12"

# codesign finds the identity through the user keychain search list.
existing="$(security list-keychains -d user | tr -d '"' | xargs)"
# shellcheck disable=SC2086
security list-keychains -d user -s "$keychain" $existing

# Use the SHA-1 hash rather than the name: it is unambiguous and the name holds no Apple team id.
identity="$(security find-identity -p codesigning "$keychain" | awk '/^ *1\) [0-9A-F]{40} / { print $2; exit }')"
if [[ -z "$identity" ]]; then
  echo '::error::No code signing identity found in the self-signed certificate.'
  exit 1
fi
echo "Using self-signed identity: $identity"
echo "APPLE_SIGNING_IDENTITY=$identity" >> "$GITHUB_ENV"
