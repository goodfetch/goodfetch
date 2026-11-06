#!/usr/bin/env bash
# The Kennel's part of a release (ADRs 0039, 0094 and 0141), in ADR 0039's
# order: build, then sign and notarize, then the files and their digests,
# then the gate. It publishes nothing: publishing the folder's files
# beside the Products' is the release pipeline's step, from the Products'
# first publish, never before.
#
#   kennel/scripts/release.sh build <folder> [<target>]
#       the Kennel's release executable for <target> (this Mac's by
#       default), from kennel/ under the Kennel's own version, in
#       <folder>/<target>/Kennel.app, signed ad hoc (bundle.sh); a build
#       job, which never sees a signing credential, runs it;
#   kennel/scripts/release.sh sign <folder>
#       each <folder>/<target>/Kennel.app signed with Developer ID and the
#       hardened runtime, notarized, stapled and verified, in the Products'
#       signing job with its `release` environment's credentials:
#         GOODFETCH_DEVELOPER_ID_P12           the Developer ID Application
#                                              certificate's .p12, in base64
#         GOODFETCH_DEVELOPER_ID_P12_PASSWORD  its password
#         GOODFETCH_NOTARY_KEY                 the App Store Connect API key
#                                              (.p8, its PEM text)
#         GOODFETCH_NOTARY_KEY_ID              its key ID
#         GOODFETCH_NOTARY_ISSUER_ID           its issuer ID
#       With none of them set, the step is inert: it says so, leaves the
#       ad hoc signature and succeeds, until the Goodfetch entity's Apple
#       Developer account exists (ADR 0039). With some but not all, it
#       fails and names the missing ones;
#   kennel/scripts/release.sh package <folder> <base URL>
#       each app in <folder>/goodfetch-kennel-<version>-<target>.zip, and
#       <folder>/install-kennel.sh, the install script stamped with each
#       archive's name and SHA-256 and the URL the release serves them
#       from;
#   kennel/scripts/release.sh verify <folder>
#       the Platform signature gate (ADR 0039) on each app before any
#       publish: `codesign --verify --strict --deep`, Developer ID and the
#       hardened runtime, `spctl --assess --type execute` and
#       `stapler validate`. An app signed ad hoc fails it.
#
# It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
usage="usage: kennel/scripts/release.sh build <folder> [<target>] | sign <folder> | package <folder> <base URL> | verify <folder>"
credentials="GOODFETCH_DEVELOPER_ID_P12 GOODFETCH_DEVELOPER_ID_P12_PASSWORD GOODFETCH_NOTARY_KEY GOODFETCH_NOTARY_KEY_ID GOODFETCH_NOTARY_ISSUER_ID"

fail() {
  echo "error: $*" >&2
  exit 1
}

# Each <folder>/<target>/Kennel.app, one a line.
apps() {
  local folder=$1 app found=false
  for app in "$folder"/*/Kennel.app; do
    [[ -d "$app" ]] || continue
    found=true
    printf '%s\n' "$app"
  done
  $found || fail "no <target>/Kennel.app in $folder: run release.sh build first"
}

build() {
  local folder=${1:?$usage} target=${2:-}
  if [[ -z "$target" ]]; then
    # The Kennel's own toolchain pin, from its folder.
    target=$(cd "$root/kennel" && rustc -vV | sed -n 's/^host: //p')
  fi
  case "$target" in
    *-apple-darwin) ;;
    *) fail "the Kennel's release app is built for macOS targets only, not $target" ;;
  esac
  (cd "$root/kennel" && cargo build --locked --release --target "$target" -p goodfetch-kennel-app)
  "$root/kennel/scripts/bundle.sh" "$root/kennel/target/$target/release/goodfetch-kennel" "$folder/$target"
  echo "Built $folder/$target/Kennel.app, signed ad hoc"
}

verify_app() {
  local app=$1 details
  codesign --verify --strict --deep "$app" || fail "$app's signature does not verify"
  details=$(codesign --display --verbose=2 "$app" 2>&1)
  grep -q '^Authority=Developer ID Application: ' <<<"$details" ||
    fail "$app is not signed with Developer ID ($(grep '^Signature=' <<<"$details" || echo "another authority")): a release publishes only the Kennel's app signed with Developer ID and notarized (ADR 0039)"
  grep -q '^CodeDirectory .*(runtime)' <<<"$details" ||
    fail "$app is signed without the hardened runtime (ADR 0039)"
  spctl --assess --type execute "$app" || fail "Gatekeeper refuses $app"
  xcrun stapler validate "$app" || fail "$app carries no notarization ticket"
  echo "Verified $app: Developer ID, the hardened runtime, notarized and stapled"
}

verify() {
  local folder=${1:?$usage} list app
  list=$(apps "$folder")
  while IFS= read -r app; do verify_app "$app"; done <<<"$list"
}

# Sets the user's keychain search list to the arguments, then the paths
# stdin holds, one a line.
keychains() {
  local list=("$@") path
  while IFS= read -r path; do list+=("$path"); done
  # bash 3.2 calls an empty array's expansion unbound.
  [[ ${#list[@]} -gt 0 ]] || return 0
  security list-keychains -d user -s "${list[@]}"
}

clean_up_signing() {
  keychains <"$work/previous-keychains" || true
  security delete-keychain "$keychain" >/dev/null 2>&1 || true
  rm -rf "$work"
}

sign() {
  local folder=${1:?$usage} name present='' missing=''
  for name in $credentials; do
    if [[ -n "${!name:-}" ]]; then present="$present $name"; else missing="$missing $name"; fi
  done
  if [[ -z "$present" ]]; then
    echo "release.sh sign: inert, since none of the release environment's signing credentials ($credentials) is set: the apps keep their ad hoc signature, which no release publishes (ADR 0039)" >&2
    return 0
  fi
  [[ -z "$missing" ]] || fail "the signing credentials lack$missing (set:$present)"

  local password identity app status
  # Globals, for the clean-up at exit: the temporary keychain leaves the
  # user's search list, then the disk.
  work=$(mktemp -d)
  keychain="$work/signing.keychain-db"
  security list-keychains -d user | tr -d '"' | sed 's/^ *//; /^$/d' >"$work/previous-keychains"
  trap clean_up_signing EXIT
  password=$(uuidgen)

  printf '%s' "$GOODFETCH_DEVELOPER_ID_P12" | base64 --decode >"$work/developer-id.p12"
  printf '%s\n' "$GOODFETCH_NOTARY_KEY" >"$work/notary.p8"
  security create-keychain -p "$password" "$keychain"
  security set-keychain-settings -lut 21600 "$keychain"
  security unlock-keychain -p "$password" "$keychain"
  security import "$work/developer-id.p12" -k "$keychain" -P "$GOODFETCH_DEVELOPER_ID_P12_PASSWORD" -T /usr/bin/codesign >/dev/null
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" >/dev/null
  keychains "$keychain" <"$work/previous-keychains"
  identity=$(security find-identity -v -p codesigning "$keychain" | awk '/"Developer ID Application: / { print $2; exit }')
  [[ -n "$identity" ]] || fail "GOODFETCH_DEVELOPER_ID_P12 holds no Developer ID Application identity"

  apps "$folder" >"$work/apps"
  while IFS= read -r app; do
    codesign --force --options runtime --timestamp --keychain "$keychain" --sign "$identity" "$app"
    rm -f "$work/notarize.zip"
    ditto -c -k --keepParent "$app" "$work/notarize.zip"
    xcrun notarytool submit "$work/notarize.zip" --key "$work/notary.p8" \
      --key-id "$GOODFETCH_NOTARY_KEY_ID" --issuer "$GOODFETCH_NOTARY_ISSUER_ID" \
      --wait --output-format json >"$work/notary.json"
    status=$(plutil -extract status raw -o - "$work/notary.json")
    [[ "$status" == Accepted ]] ||
      fail "the notary service answered $status for $app (submission $(plutil -extract id raw -o - "$work/notary.json"); read its log with xcrun notarytool log)"
    xcrun stapler staple "$app"
    verify_app "$app"
  done <"$work/apps"
}

package() {
  local folder=${1:?$usage} base_url=${2:?$usage} list app target version='' this archive digest lines=''
  list=$(apps "$folder")
  while IFS= read -r app; do
    target=$(basename "$(dirname "$app")")
    this=$(plutil -extract CFBundleShortVersionString raw -o - "$app/Contents/Info.plist")
    [[ -z "$version" || "$version" == "$this" ]] || fail "the apps carry two versions, $version and $this"
    version=$this
    archive="goodfetch-kennel-$version-$target.zip"
    rm -f "$folder/$archive"
    ditto -c -k --keepParent "$app" "$folder/$archive"
    digest=$(shasum -a 256 "$folder/$archive" | awk '{ print $1 }')
    lines="$lines$target $archive $digest"$'\n'
    echo "Packaged $folder/$archive"
  done <<<"$list"

  local template="$root/kennel/scripts/install-kennel.sh" script="$folder/install-kennel.sh"
  {
    sed -n '1,/^# >>> release/p' "$template"
    printf 'version=%q\n' "$version"
    printf 'base_url=%q\n' "${base_url%/}"
    echo '# One line per target: <Rust target> <archive> <SHA-256>.'
    printf 'archives=%q\n' "${lines%$'\n'}"
    sed -n '/^# <<< release/,$p' "$template"
  } >"$script"
  chmod 755 "$script"
  echo "Wrote $script, for the Kennel $version from $base_url"
}

part=${1:?$usage}
shift
case "$part" in
  build | sign | package | verify) "$part" "$@" ;;
  *) fail "$usage" ;;
esac
