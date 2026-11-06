#!/usr/bin/env bash
# The workspace-hack crate, which gives every package selection one feature
# resolution of the build side (ADR 0094's note of 2026-11-06, #2089).
#
#   ci/workspace-hack.sh generate   writes it with cargo-hakari
#   ci/workspace-hack.sh check      fails when it is out of date (`just lint`)
#
# cargo-hakari (.config/hakari.toml) lists, for every shared dependency
# that the workspace's build scripts and proc-macros (the build side) use
# with more than one feature set, the union of those sets, and makes every
# member depend on `goodfetch-workspace-hack`. Each selection (`-p a -p b`,
# `xtask affected`'s, a test build's) then builds every build script with
# the same dependencies, so a build script runs once per profile and the C
# and C++ that DuckDB's, BoringSSL's and the other `-sys` crates' scripts
# compile is reused across selections.
#
# Only the build side's tables are kept. cargo-hakari also unifies the code
# side, but its union takes the dev-dependencies' features and crates
# (Tokio's `test-util`, proptest) and Foxhound's (Arrow), which every
# member, Collie's executable among them, would then build and link
# (ADRs 0087 and 0094). The code side keeps one resolution per selection,
# with no C or C++ in what differs.
#
# `goodfetch-protocol` is left out (.config/hakari.toml's final-excludes):
# the Kennel's workspace links it by path.
#
# `cargo hakari verify` checks both sides, so the check here is the
# regeneration's diff instead, and `manage-deps --dry-run` for the members'
# dependency lines.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
manifest=$root/crates/goodfetch-workspace-hack/Cargo.toml

# Drops the code side's tables (`[dependencies]` and
# `[target.'cfg(…)'.dependencies]`) from hakari's section.
keep_build_side() {
  awk '
    /^### BEGIN HAKARI SECTION/ { inside = 1; drop = 0; print; next }
    /^### END HAKARI SECTION/ { inside = 0; drop = 0; print; next }
    inside && /^\[/ { drop = ($0 == "[dependencies]" || $0 ~ /^\[target\..*\.dependencies\]$/) }
    !drop { print }
  ' "$manifest" >"$manifest.tmp"
  mv "$manifest.tmp" "$manifest"
}

generate() {
  (cd "$root" && cargo hakari generate --quiet && cargo hakari manage-deps --quiet --yes)
  keep_build_side
}

case "${1:-}" in
  generate)
    generate
    ;;
  check)
    saved=$(mktemp)
    cp "$manifest" "$saved"
    trap 'cp "$saved" "$manifest"; rm -f "$saved"' EXIT
    (cd "$root" && cargo hakari generate --quiet)
    keep_build_side
    failed=0
    if ! diff -u "$saved" "$manifest"; then
      echo "error[workspace-hack] crates/goodfetch-workspace-hack/Cargo.toml is out of date: run ci/workspace-hack.sh generate" >&2
      failed=1
    fi
    if ! (cd "$root" && cargo hakari manage-deps --dry-run --quiet); then
      echo "error[workspace-hack] a member does not depend on goodfetch-workspace-hack: run ci/workspace-hack.sh generate" >&2
      failed=1
    fi
    exit "$failed"
    ;;
  *)
    echo "usage: ci/workspace-hack.sh generate | check" >&2
    exit 2
    ;;
esac
