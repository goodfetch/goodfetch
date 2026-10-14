#!/usr/bin/env bash
# Fill the git-ignored `.env` with the tokens listed in `.env.example` (ADR 0076).
#
#   scripts/env-setup.sh           ask for each value; Enter keeps the current one, "-" clears it
#   scripts/env-setup.sh --check   list which names are set, without printing any value
#
# Values are read without echo, never printed, and written to `.env` with mode 600.
# Names already in `.env` but not in `.env.example` are kept as they are.
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "$root"
example=".env.example"
target=".env"

[ -f "$example" ] || { echo "error: $example not found" >&2; exit 1; }
if git ls-files --error-unmatch "$target" >/dev/null 2>&1; then
  echo "error: $target is tracked by git; remove it from the index before storing secrets" >&2
  exit 1
fi
git check-ignore -q "$target" || { echo "error: $target is not git-ignored" >&2; exit 1; }

# Current value of NAME in .env, without surrounding quotes (empty when absent).
current() {
  [ -f "$target" ] || return 0
  local line
  line="$(grep -E "^$1=" "$target" | tail -n 1 || true)"
  line="${line#*=}"
  case "$line" in
    \'*\') line="${line#\'}"; line="${line%\'}" ;;
    \"*\") line="${line#\"}"; line="${line%\"}" ;;
  esac
  printf '%s' "$line"
}

# Names and their comment line from .env.example, in order: "NAME<TAB>comment".
entries() {
  awk '/^# / { c = substr($0, 3); next }
       /^[A-Z][A-Z0-9_]*=/ { split($0, a, "="); print a[1] "\t" c; c = "" }' "$example"
}

if [ "${1:-}" = "--check" ]; then
  entries | while IFS=$'\t' read -r name comment; do
    if [ -n "$(current "$name")" ]; then state="set"; else state="empty"; fi
    printf '%-32s %-6s %s\n' "$name" "$state" "$(printf '%s' "$comment" | sed -n 's/.*needed from: \([^|]*\).*/\1/p')"
  done
  exit 0
fi
[ -z "${1:-}" ] || { echo "usage: scripts/env-setup.sh [--check]" >&2; exit 2; }

umask 077
tmp="$(mktemp "$root/.env.XXXXXX")"
trap 'rm -f "$tmp"' EXIT
{
  echo "# Written by scripts/env-setup.sh. Never commit this file (ADR 0076)."
  echo
} > "$tmp"

names=""
while IFS=$'\t' read -r name comment <&3; do
  names="$names $name"
  what="${comment%% | *}"
  rest="${comment#* | }"
  old="$(current "$name")"
  if [ -n "$old" ]; then state="currently set"; else state="currently empty"; fi
  echo
  echo "$name ($state)"
  echo "  $what"
  printf '%s\n' "$rest" | tr '|' '\n' | sed 's/^ */  /'
  printf '  value (Enter keeps, - clears): '
  if ! IFS= read -rs value; then
    echo
    echo "error: input ended before a value for $name; $target not changed" >&2
    exit 1
  fi
  echo
  case "$value" in
    "") value="$old" ;;
    -) value="" ;;
    *"'"*|*'"'*|*"\\"*|*$'\n'*) echo "error: $name contains a quote, a backslash or a newline; kept the current value" >&2; value="$old" ;;
  esac
  printf "%s='%s'\n" "$name" "$value" >> "$tmp"
done 3< <(entries)

# Keep names that .env has but .env.example does not list.
if [ -f "$target" ]; then
  { grep -E '^[A-Z][A-Z0-9_]*=' "$target" || true; } | while IFS= read -r line; do
    case " $names " in *" ${line%%=*} "*) ;; *) printf '%s\n' "$line" ;; esac
  done >> "$tmp"
fi

mv "$tmp" "$target"
trap - EXIT
chmod 600 "$target"
echo
echo "Wrote $target (mode 600). Check with: scripts/env-setup.sh --check"
