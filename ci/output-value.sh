# shellcheck shell=bash
# The one reader of the `<key>=<value>` lines xtask gives the scripts:
# `xtask affected`'s selection, `xtask gate plan`'s plan and `xtask gate
# steps`' step list. Sourced by the `ci/` scripts and the justfile's
# recipes that read them.
#
#   output_value <key> <output>   prints the value of <output>'s one
#                                 `<key>=` line
#
# It fails, naming the key, when no line or more than one has it, so a
# renamed or missing key stops its reader instead of handing it an empty
# value. A reader takes the value in an assignment of its own,
# `value=$(output_value …)`, which `set -e` stops on, never inside a test
# or a here-string, which hide the failure.
#
# It keeps to bash 3.2, which stock macOS runs.
output_value() {
  local key=$1 output=$2 line value="" found=0
  while IFS= read -r line; do
    if [[ $line == "$key="* ]]; then
      value=${line#"$key="}
      found=$((found + 1))
    fi
  done <<<"$output"
  if [[ $found -ne 1 ]]; then
    echo "error: the output has $found \`$key=\` lines, not one; the script and the xtask that wrote it disagree on its keys:" >&2
    printf '%s\n' "$output" | head -n 20 >&2
    return 1
  fi
  printf '%s\n' "$value"
}
