#!/usr/bin/env bash
# The security advisories of each ecosystem's locked dependencies (STYLE.md,
# Dependencies: no open security advisory). Each tool reads a public advisory
# database, so `just audit` runs before ADR 0043's outbound-traffic deny, in
# a job that runs none of the product. All free; no paid scanner is used
# (ADR 0076).
#
#   ci/audit.sh rust [<workspace Cargo.toml>]   cargo-deny's advisories (RustSec) and cargo-audit
#   ci/audit.sh npm [<pnpm workspace folder>]   pnpm audit (the npm registry's advisories)
#   ci/audit.sh python [<requirements file>]    pip-audit (the PyPI and OSV advisories)
#
# Without a target, each audits the repository's own lockfile; the proofs
# (`just proofs`) give each a fixture that must fail.
set -euo pipefail

repository=$(cd "$(dirname "$0")/.." && pwd)
ecosystem=${1:?the ecosystem: rust, npm or python}

case "$ecosystem" in
  rust)
    manifest=${2:-$repository/Cargo.toml}
    lockfile="$(dirname "$manifest")/Cargo.lock"
    failed=0
    cargo deny --manifest-path "$manifest" --config "$repository/deny.toml" check advisories || failed=1
    # Unmaintained, unsound and yanked crates fail here as in cargo-deny.
    cargo audit --file "$lockfile" --deny warnings || failed=1
    exit "$failed"
    ;;
  npm)
    pnpm audit --dir "${2:-$repository}" --audit-level low
    ;;
  python)
    requirements=${2:-}
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
