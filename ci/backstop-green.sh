#!/usr/bin/env bash
# The backstop's green runs (ADR 0135), which ci/merge-cap.sh counts merges from
# and ci/gate-backstop.sh checks earned passes from: its record is
# `<log>/results.txt`, and a green run is a line `<day> <commit> gate=pass
# misses=0 …`. Sourced. It keeps to bash 3.2, which stock macOS runs.

# The backstop's switch (ADR 0135): `ci/gate-backstop.state` holds one word,
# `on` or `off`. ci/gate-backstop.sh reads the one of the commit it tests,
# and ci/merge-cap.sh the one `origin/main` holds. Prints the word in $1, the
# file's content; a commit without the file, from before the switch, is `on`.
# Any other word fails: a switch is not guessed at.
backstop_state() {
  case ${1:-on} in
    on | off) echo "${1:-on}" ;;
    *)
      echo "error: ci/gate-backstop.state says \`$1\`, which is neither \`on\` nor \`off\`" >&2
      return 1
      ;;
  esac
}

# Prints the commit of each green run in the results file $1, oldest first.
backstop_greens() {
  awk '$3 == "gate=pass" && $4 == "misses=0" { print $2 }' "$1" 2>/dev/null || true
}

# Prints the latest of the commits $1, one a line, that the ref $2 holds, or
# nothing.
latest_green() {
  local commit tested=""
  while IFS= read -r commit; do
    if [[ -n $commit ]] && git merge-base --is-ancestor "$commit" "$2" 2>/dev/null; then
      tested=$commit
    fi
  done <<<"$1"
  echo "$tested"
}
