#!/usr/bin/env bash
# Reads ci/runtimes.toml, the runtime matrix of the SDKs' tests (ADR 0069),
# for .github/actions/host-tools and ci/host/fetch.sh:
#   ids <set>      each runtime ID of the set, one a line, in its order;
#   version <id>   the runtime's pinned version, or nothing for one without;
#   outputs <set>  `<id>=<version>` for each pinned runtime of the set, the
#                  step outputs host-tools installs from.
# The matrix keeps each runtime and each set on one line (`xtask runtimes`
# checks it), so a line-by-line read is enough. It keeps to bash 3.2, which
# stock macOS runs on the owner's machine.
set -euo pipefail

file="$(cd "$(dirname "$0")/../.." && pwd)/ci/runtimes.toml"

ids() {
  awk -v key="$1 = [" '
    index($0, key) == 1 {
      found = 1
      sub(/^[^[]*\[/, ""); sub(/\][[:space:]]*$/, ""); gsub(/[" ]/, "")
      count = split($0, ids, ",")
      for (i = 1; i <= count; i++) if (ids[i] != "") print ids[i]
    }
    END { exit !found }
  ' "$file" || {
    echo "error: $file has no set $1" >&2
    exit 2
  }
}

version() {
  awk -v bare="$1 = {" -v quoted="\"$1\" = {" '
    (index($0, bare) == 1 || index($0, quoted) == 1) && match($0, /version = "[^"]*"/) {
      print substr($0, RSTART + 11, RLENGTH - 12)
    }
  ' "$file"
}

command=${1:-}
case "$command" in
  ids) ids "${2:?usage: ci/host/runtimes.sh ids <set>}" ;;
  version) version "${2:?usage: ci/host/runtimes.sh version <id>}" ;;
  outputs)
    set_ids=$(ids "${2:?usage: ci/host/runtimes.sh outputs <set>}")
    for id in $set_ids; do
      pin=$(version "$id")
      if [[ -n "$pin" ]]; then echo "$id=$pin"; fi
    done
    ;;
  *)
    echo "usage: ci/host/runtimes.sh ids|version|outputs <set or id>" >&2
    exit 2
    ;;
esac
