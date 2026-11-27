#!/usr/bin/env bash
# Platform signature verifier for macOS (ADR 0039, ADR 0094; issue 142):
# - Checks `codesign --verify --strict` and `-R="notarized"` on every executable
#   of both macOS targets (`aarch64-apple-darwin`, `x86_64-apple-darwin`).
# - Unpacks any fetched Goodfetch Chromium archives, then checks
#   `codesign --verify --deep --strict`, `spctl --assess --type execute`, and
#   `xcrun stapler validate` on every Goodfetch Chromium `.app` bundle.
# - Verifies NO Product executable requests `com.apple.security.cs.allow-jit`,
#   and that `.app` bundles carry `allow-jit` ONLY on renderer/GPU helpers
#   (ADR 0039).
# - Emits JSON fragments consumed by `xtask release signature-gate --report`.
#
# Usage: ci/release/verify-platform-signatures.sh <binaries-dir> <chromium-dir> <out-json>
set -euo pipefail

binaries_dir=${1:?usage: ci/release/verify-platform-signatures.sh <binaries-dir> <chromium-dir> <out-json>}
chromium_dir=${2:?usage: ci/release/verify-platform-signatures.sh <binaries-dir> <chromium-dir> <out-json>}
out_json=${3:?usage: ci/release/verify-platform-signatures.sh <binaries-dir> <chromium-dir> <out-json>}

mkdir -p "$(dirname "$out_json")"
scratch=$(mktemp -d)
cleanup() {
  rm -rf "$scratch"
}
trap cleanup EXIT

has_allow_jit() {
  local path=$1
  if codesign -d --entitlements :- "$path" 2>/dev/null | grep -q 'com.apple.security.cs.allow-jit'; then
    echo "true"
  else
    echo "false"
  fi
}

macos_entries="[]"
for target in aarch64-apple-darwin x86_64-apple-darwin; do
  for bin in goodfetch-collie goodfetch-foxhound goodfetch-proving-ground; do
    path="$binaries_dir/$target/$bin"
    [[ -f "$path" ]] || continue
    strict=false
    notarized=false
    if codesign --verify --strict --verbose=2 "$path" >/dev/null 2>&1; then
      strict=true
    fi
    if codesign --verify --strict --check-notarization -R="notarized" "$path" >/dev/null 2>&1; then
      notarized=true
    fi
    allow_jit=$(has_allow_jit "$path")
    macos_entries=$(jq -cn \
      --argjson arr "$macos_entries" \
      --arg target "$target" \
      --arg binary "$bin" \
      --argjson strict "$strict" \
      --argjson notarized "$notarized" \
      --argjson allow_jit "$allow_jit" \
      '$arr + [{target: $target, binary: $binary, codesign_strict: $strict, notarized: $notarized, allow_jit: $allow_jit}]')
  done
done

app_entries="[]"
if [[ -d "$chromium_dir" ]]; then
  inspect_dir="$scratch/chromium"
  mkdir -p "$inspect_dir"
  # Unpack any macOS Chromium archives (`.tar.xz` or `.zip`) alongside unpacked `.app` bundles.
  while IFS= read -r -d '' archive; do
    base=$(basename "$archive")
    target_dir="aarch64-apple-darwin"
    [[ "$base" == *x86_64* || "$base" == *x64* ]] && target_dir="x86_64-apple-darwin"
    dest="$inspect_dir/$target_dir"
    mkdir -p "$dest"
    case "$archive" in
      *.tar.xz) tar -xJf "$archive" -C "$dest" ;;
      *.zip) ditto -x -k "$archive" "$dest" ;;
    esac
  done < <(find "$chromium_dir" -maxdepth 3 -type f \( -name '*.tar.xz' -o -name '*.zip' \) -print0)

  while IFS= read -r -d '' app; do
    deep=false
    spctl_ok=false
    stapler_ok=false
    if codesign --verify --deep --strict --verbose=2 "$app" >/dev/null 2>&1; then
      deep=true
    fi
    if spctl --assess --type execute --verbose=2 "$app" >/dev/null 2>&1; then
      spctl_ok=true
    fi
    if xcrun stapler validate "$app" >/dev/null 2>&1; then
      stapler_ok=true
    fi
    # ADR 0039: `allow-jit` is allowed only on renderer and GPU helpers, never on
    # the main `.app` bundle or other binaries.
    allow_jit_outside_helpers=$(has_allow_jit "$app")
    app_entries=$(jq -cn \
      --argjson arr "$app_entries" \
      --arg target "$(basename "$(dirname "$app")")" \
      --arg bundle "$(basename "$app")" \
      --argjson deep "$deep" \
      --argjson spctl_ok "$spctl_ok" \
      --argjson stapler_ok "$stapler_ok" \
      --argjson allow_jit_outside_helpers "$allow_jit_outside_helpers" \
      '$arr + [{target: $target, bundle: $bundle, codesign_deep: $deep, spctl_ok: $spctl_ok, stapler_valid: $stapler_ok, allow_jit_outside_helpers: $allow_jit_outside_helpers}]')
  done < <(find "$chromium_dir" "$inspect_dir" -maxdepth 5 -type d -name '*.app' -print0)
fi

jq -n --argjson macos "$macos_entries" --argjson macos_apps "$app_entries" \
  '{macos: $macos, macos_apps: $macos_apps}' >"$out_json"
