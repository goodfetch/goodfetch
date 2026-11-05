#!/usr/bin/env bash
# The security advisories of each ecosystem's locked dependencies (STYLE.md,
# Dependencies: no open security advisory), in two modes (ADR 0064):
#
# - the gate's, by default (`just audit`): against the advisory snapshot
#   ci/advisories.toml pins, which `xtask advisories fetch` keeps in
#   target/advisories/ (downloading only what is missing or differs from its
#   pin, through GitHub's API with GITHUB_TOKEN or `gh auth token`), every
#   tool offline. Its verdict moves only with a commit: a fail is an
#   advisory in the snapshot against what the commit locks.
# - `live` (`just audit-live`, the daily Live check on `main`): against each
#   ecosystem's newest public advisories, each tool fetching its own.
#
# All free; no paid scanner is used (ADR 0076).
#
#   ci/audit.sh rust [<workspace Cargo.toml>]   cargo-deny's advisories and cargo-audit, over RustSec's pinned commit
#   ci/audit.sh npm [<pnpm-lock.yaml>]           OSV-Scanner, over OSV's pinned npm database
#   ci/audit.sh python [<uv.lock>]               OSV-Scanner, over OSV's pinned PyPI database
#   ci/audit.sh live rust [<workspace Cargo.toml>]      cargo-deny's advisories and cargo-audit, over RustSec's newest
#   ci/audit.sh live npm [<pnpm workspace folder>]      pnpm audit (the npm registry's advisories)
#   ci/audit.sh live python [<requirements file>]       pip-audit (the PyPI and OSV advisories)
#
# Without a target, each audits the repository's own lockfile; the proofs
# (`just proofs` and ci/proofs/live.sh) give each a fixture that must fail.
set -euo pipefail

repository=$(cd "$(dirname "$0")/.." && pwd)
mode=pinned
if [[ ${1:-} == live ]]; then
  mode=live
  shift
fi
ecosystem=${1:?the ecosystem: rust, npm or python}
target=${2:-}
advisories="$repository/target/advisories"
# Where cargo-deny 0.20 looks for RustSec's database under its db-path
# (deny.toml reads GOODFETCH_ADVISORY_DBS), named after its default URL.
rustsec_folder=advisory-db-3157b0e258782691

# Keeps the pinned snapshot; a no-op, offline, while it is kept as pinned.
fetch() {
  (cd "$repository" &&
    GITHUB_TOKEN="${GITHUB_TOKEN:-$(gh auth token 2>/dev/null || true)}" \
      cargo run --quiet --bin xtask -- advisories fetch)
}

# OSV-Scanner over one lockfile, offline, with OSV's pinned database of
# `ecosystem`, laid out where OSV-Scanner 2.7 reads its offline databases
# (`<cache folder>/osv-scalibr/<ecosystem>/all.zip`). It exits non-zero on
# any advisory, and on a database it cannot find.
osv() {
  local ecosystem=$1 lockfile=$2
  local cache="$advisories/osv-scanner"
  mkdir -p "$cache/osv-scalibr/$ecosystem"
  ln -sfn "$advisories/osv/$ecosystem.zip" "$cache/osv-scalibr/$ecosystem/all.zip"
  OSV_SCANNER_LOCAL_DB_CACHE_DIRECTORY="$cache" \
    osv-scanner scan source --offline --lockfile "$lockfile"
}

case "$mode-$ecosystem" in
  pinned-rust | live-rust)
    manifest=${target:-$repository/Cargo.toml}
    folder=$(cd "$(dirname "$manifest")" && pwd)
    lockfile="$folder/Cargo.lock"
    # A lockfile outside the workspace, such as Spider's side of `just
    # compare-spider` (xtask/compare-spider), keeps its own advisory
    # exceptions beside it when it has any: a deny.toml of its own and a
    # .cargo/audit.toml, which cargo-audit reads from its working folder, so
    # they never reach the workspace's.
    config="$repository/deny.toml"
    audit_folder=$repository
    [[ -f "$folder/deny.toml" ]] && config="$folder/deny.toml"
    [[ -f "$folder/.cargo/audit.toml" ]] && audit_folder=$folder
    failed=0
    if [[ $mode == pinned ]]; then
      fetch
      # cargo-deny's --offline also keeps cargo metadata offline, and
      # cargo-audit's --stale accepts the pinned commit's age: the daily
      # live audit reads the newest.
      GOODFETCH_ADVISORY_DBS="$advisories/rustsec" cargo deny --offline --manifest-path "$manifest" \
        --config "$config" check advisories || failed=1
      # Unmaintained, unsound and yanked crates fail here as in cargo-deny.
      (cd "$audit_folder" && cargo audit --db "$advisories/rustsec/$rustsec_folder" --no-fetch --stale \
        --file "$lockfile" --deny warnings) || failed=1
    else
      GOODFETCH_ADVISORY_DBS="${CARGO_HOME:-$HOME/.cargo}/advisory-dbs" cargo deny --manifest-path "$manifest" \
        --config "$config" check advisories || failed=1
      (cd "$audit_folder" && cargo audit --file "$lockfile" --deny warnings) || failed=1
    fi
    exit "$failed"
    ;;
  pinned-npm)
    fetch
    osv npm "${target:-$repository/pnpm-lock.yaml}"
    ;;
  pinned-python)
    fetch
    osv PyPI "${target:-$repository/uv.lock}"
    ;;
  live-npm)
    pnpm audit --dir "${target:-$repository}" --audit-level low
    ;;
  live-python)
    requirements=$target
    if [[ -z "$requirements" ]]; then
      mkdir -p "$repository/target"
      requirements="$repository/target/requirements-audit.txt"
      (cd "$repository" && uv export --frozen --all-packages --no-emit-workspace \
        --format requirements-txt --quiet --output-file "$requirements")
    fi
    (cd "$repository" && uv run --frozen pip-audit --require-hashes --disable-pip \
      --progress-spinner off --requirement "$requirements")
    ;;
  *)
    echo "error: the ecosystem is rust, npm or python, not $ecosystem" >&2
    exit 2
    ;;
esac
