#!/usr/bin/env bash
# Refuses Rust module structure that deslop-please flags (`just deslop-please`,
# run by a spec's implementer at the spec's end and by the full gate, never the
# quick gate; TESTING.md, Quality register, Floor):a cycle of modules that use each
# other (`module-cycle`), items of a module and of its child that know each
# other (`tangle`), a struct field used outside its owner (`field-leak`), and
# a module whose callers use most of its functions (`shallow-module`).
# ci/deslop-please.allow lists the findings `main` and the spec branches open
# on 2026-11-26 held when the check came, each by deslop-please's key, which
# names modules and items, not lines. The check fails on a finding the list
# does not hold. A listed finding the tree no longer holds is a note, not a
# failure, so a spec branch the list was taken from stays green; remove it
# with `--shrink`. Spec #4005 fixes the listed findings crate by crate.
#
#   ci/deslop-please.sh            the check
#   ci/deslop-please.sh --shrink   removes from the list the findings that are
#                                  gone (`just deslop-please-shrink`); it never
#                                  adds one
#
# Left out: `long-file` and `wide-directory`, which deslop-please also
# reports. They are caps on a file's lines and a folder's entries, and ADR
# 0126 rejected a cap in favour of its `Size:` alarm (ADR 0126's note of
# 2026-11-26).
#
# The list and this script are files of the gate (TESTING.md, Floor): a
# commit that edits either carries a `Quality-change` trailer naming it.
# The check reads the tree alone, never the change's base, as the gate
# reuses a green lint on the same tree.
set -euo pipefail
cd "$(dirname "$0")/.."

list=ci/deslop-please.allow
ignored='["long-file", "wide-directory"]'
gated=".diagnostics[] | select(.lint as \$lint | \$ignored | index(\$lint) | not)"

# deslop-please exits 1 whenever it reports a finding, with its JSON still
# whole; a run that wrote nothing failed on its own.
if [[ ${1:-} == --shrink ]]; then
  report=$(deslop-please --json) || [[ -n ${report:-} ]] || exit 1
  current=$(jq -r --argjson ignored "$ignored" "$gated"' | "\(.lint) \(.key)"' <<< "$report" | sort -u)
  kept=$(grep -v '^#' "$list" | sort -u | comm -12 - <(printf '%s\n' "$current"))
  {
    grep '^#' "$list"
    [[ -z $kept ]] || printf '%s\n' "$kept"
  } > "$list.new"
  mv "$list.new" "$list"
  echo "$list: $(grep -vc '^#' "$list") findings kept."
  exit 0
fi

report=$(deslop-please --allow "$list" --json) || [[ -n ${report:-} ]] || exit 1
new=$(jq -r --argjson ignored "$ignored" "$gated"' | "error[\(.lint)]: \(.message)\n  --> \(.file // "?"):\(.line // 0)\n  key: \(.key)" + ((.notes // []) | map("\n  = note: " + .) | join(""))' <<< "$report")
gone=$(jq -r '.fixed_allowed' <<< "$report")

if [[ $gone != 0 ]]; then
  echo "note: $gone findings of $list are gone from the tree; \`just deslop-please-shrink\` removes them."
fi
if [[ -n $new ]]; then
  printf '%s\n' "$new"
  echo "error: deslop-please found module structure $list does not list. Fix it at the module's interface, not by a move that only hides it."
  exit 1
fi
