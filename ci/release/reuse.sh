#!/usr/bin/env bash
# `ci/release/reuse.sh <repository>`: the Release suite's Linux x64 glibc
# cell reuses the Pre-merge suite's units that a batch's hosted run passed
# on the tree it tests (#3384, ADR 0135's note of 2026-11-23), before the
# deny. The hosted Pre-merge jobs build and run each crate's tests on the
# same target, with the same nextest profile and Cargo profile as this
# cell's Rust step, so a crate they passed on this tree runs here only when
# the Release suite selects its tests otherwise.
#
# It downloads the records of the pull-request workflow's runs on the
# checked-out commit and on its parents of the same tree, from
# `repository`'s pull requests alone (`ci/hosted-records.sh tree`), and
# plans the Pre-merge suite by the layer `release` from them and no record of
# its own (`xtask gate plan --layer release`). The plan reruns every member
# that is no crate, whose tests the cell runs on each Node line and each
# CPython, each crate a test of whose reads the running suite, and each unit
# no hosted run passed on this very tree. Its log names each hosted run it
# reads, with its page, and the head each reused unit was green on.
#
# It writes `reused=<crate> …` to GITHUB_OUTPUT, which the cell's Rust step
# skips (ci/release/cell.sh rust, GOODFETCH_RELEASE_REUSED). A unit a hosted
# run went red on on this tree fails the step (ADR 0064).
#
# It keeps to bash 3.2, which stock macOS runs.
set -euo pipefail

repository=${1:?usage: ci/release/reuse.sh <repository>}
# It runs in the checkout's root, as the cell's steps do.
ci=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=ci/output-value.sh
source "$ci/output-value.sh"

folder=target/release-suite/hosted-records
records=target/release-suite/no-gate-records
rm -rf "$records"
mkdir -p "$records"
"$ci/hosted-records.sh" tree "$repository" HEAD "$folder"
plan=$(cargo run --quiet --locked --bin xtask -- gate plan --records "$records" --dry-run \
  --now "$(date -u +%s)" --layer release --head-repository "$repository" --hosted-records "$folder")
members=$(output_value members "$plan")
action=$(output_value pre-merge.action "$plan")
why=$(output_value pre-merge.why "$plan")
case $action in
  reuse) rerun="" ;;
  part) rerun=$(output_value pre-merge.members "$plan") ;;
  run) rerun=$members ;;
  red)
    echo "error: a hosted run went red on this tree, so the commit is red and its tests do not run again (ADR 0064): $why" >&2
    exit 1
    ;;
  *)
    echo "error: the plan's Pre-merge suite is $action, which the Release cell does not take" >&2
    exit 1
    ;;
esac

reused=()
for member in $members; do
  [[ " $rerun " == *" $member "* ]] || reused+=("$member")
done
if [[ ${#reused[@]} -eq 0 ]]; then
  echo "Release: no Pre-merge unit is reused, so the Rust step runs every crate's tests ($why)"
else
  echo "Release: the Rust step skips the crates a hosted run passed on this tree: ${reused[*]}"
  [[ -z $rerun ]] || echo "Release: it runs these again: $rerun ($why)"
fi
echo "reused=${reused[*]-}" >>"${GITHUB_OUTPUT:?the job sets GITHUB_OUTPUT}"
