#!/usr/bin/env bash
# Signs and notarizes macOS release executables and Goodfetch Chromium `.app`
# bundles with Apple Developer ID and hardened runtime (ADRs 0039, 0051, 0074,
# 0088 and 0094; issue 142):
# - Imports the Developer ID `.p12` into an ephemeral temporary keychain cleaned
#   up on exit (never touches the user's login keychain).
# - Uses hardened runtime (`--options runtime`) with NO `allow-jit` entitlement
#   (both Products embed V8 running `--jitless`).
# - Submits both Products' executables of each macOS target (`aarch64-apple-darwin`
#   and `x86_64-apple-darwin`) in ONE ZIP to `xcrun notarytool submit --wait`.
# - Signs Goodfetch Chromium bundles fetched by digest (never rebuilds), then
#   notarizes and staples `.app` bundles.
#
# Usage: ci/release/sign-macos.sh <mode: proving-ground|artifacts> <dir> [chromium-dir]
set -euo pipefail

mode=${1:?usage: ci/release/sign-macos.sh <proving-ground|artifacts> <dir> [chromium-dir]}
target_dir=${2:?usage: ci/release/sign-macos.sh <proving-ground|artifacts> <dir> [chromium-dir]}
chromium_dir=${3:-}

: "${APPLE_DEVELOPER_ID_P12:?missing APPLE_DEVELOPER_ID_P12 (base64)}"
: "${APPLE_DEVELOPER_ID_PASSWORD:?missing APPLE_DEVELOPER_ID_PASSWORD}"
: "${APPLE_SIGNING_IDENTITY:?missing APPLE_SIGNING_IDENTITY}"
: "${APPLE_NOTARY_KEY:?missing APPLE_NOTARY_KEY (base64 .p8)}"
: "${APPLE_NOTARY_KEY_ID:?missing APPLE_NOTARY_KEY_ID}"
: "${APPLE_NOTARY_ISSUER_ID:?missing APPLE_NOTARY_ISSUER_ID}"

scratch=$(mktemp -d)
keychain="$scratch/goodfetch-signing.keychain-db"
keychain_pw=$(openssl rand -hex 24)
p12_path="$scratch/developer-id.p12"
notary_key_path="$scratch/AuthKey_${APPLE_NOTARY_KEY_ID}.p8"

cleanup() {
  security delete-keychain "$keychain" >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap cleanup EXIT

printf '%s' "$APPLE_DEVELOPER_ID_P12" | base64 --decode >"$p12_path"
printf '%s' "$APPLE_NOTARY_KEY" | base64 --decode >"$notary_key_path"
chmod 600 "$p12_path" "$notary_key_path"

security create-keychain -p "$keychain_pw" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$keychain_pw" "$keychain"
security import "$p12_path" \
  -P "$APPLE_DEVELOPER_ID_PASSWORD" \
  -A -t cert -f pkcs12 -k "$keychain"
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_pw" "$keychain"
security list-keychains -d user -s "$keychain"

sign_binary() {
  local path=$1
  # Hardened runtime with timestamp and NO `com.apple.security.cs.allow-jit` entitlement.
  codesign --force --timestamp --options runtime \
    --keychain "$keychain" \
    --sign "$APPLE_SIGNING_IDENTITY" \
    "$path"
  codesign --verify --strict --verbose=2 "$path"
}

notarize_zip() {
  local zip_path=$1
  xcrun notarytool submit "$zip_path" \
    --key "$notary_key_path" \
    --key-id "$APPLE_NOTARY_KEY_ID" \
    --issuer "$APPLE_NOTARY_ISSUER_ID" \
    --wait
}

for target in aarch64-apple-darwin x86_64-apple-darwin; do
  dir="$target_dir/$target"
  [[ -d "$dir" ]] || continue

  if [[ "$mode" == "proving-ground" ]]; then
    pg="$dir/goodfetch-proving-ground"
    if [[ -f "$pg" ]]; then
      sign_binary "$pg"
      zip_path="$scratch/proving-ground-$target.zip"
      ditto -c -k --keepParent "$pg" "$zip_path"
      notarize_zip "$zip_path"
    fi
    continue
  fi

  # Sign both Products' executables and any first-party Plugins for this target.
  stage="$scratch/notarize-$target"
  mkdir -p "$stage"
  for exe in goodfetch-collie goodfetch-foxhound; do
    if [[ -f "$dir/$exe" ]]; then
      sign_binary "$dir/$exe"
      cp -p "$dir/$exe" "$stage/$exe"
    fi
  done
  for plugin in "$dir"/goodfetch-plugin-*; do
    [[ -f "$plugin" ]] || continue
    sign_binary "$plugin"
    cp -p "$plugin" "$stage/$(basename "$plugin")"
  done

  # ADR 0094: submit both Products' executables of one target in a single ZIP.
  zip_path="$scratch/goodfetch-$target.zip"
  ditto -c -k "$stage" "$zip_path"
  notarize_zip "$zip_path"
done

# Sign and notarize Goodfetch Chromium `.app` bundles fetched by digest (ADR 0074;
# never rebuild Chromium here).
if [[ "$mode" == "artifacts" && -n "$chromium_dir" && -d "$chromium_dir" ]]; then
  while IFS= read -r -d '' app_bundle; do
    codesign --force --deep --timestamp --options runtime \
      --keychain "$keychain" \
      --sign "$APPLE_SIGNING_IDENTITY" \
      "$app_bundle"
    app_zip="$scratch/$(basename "$app_bundle").zip"
    ditto -c -k --keepParent "$app_bundle" "$app_zip"
    notarize_zip "$app_zip"
    xcrun stapler staple "$app_bundle"
    xcrun stapler validate "$app_bundle"
  done < <(find "$chromium_dir" -maxdepth 4 -type d -name '*.app' -print0)
fi
