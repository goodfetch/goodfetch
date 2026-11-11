#!/usr/bin/env bash
# peak-cuts: rustc's linker for the musl executable. Runs cargo-zigbuild's own
# linker (zig cc) with ZIG_VERBOSE_LINK=1, then saves the ld.lld command Zig ran
# and every input file to $LINK_SAVE, so relink.sh can re-link the same objects
# with other lld flags in seconds instead of a new fat-LTO build.
set -euo pipefail
real=${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER:?cargo-zigbuild sets the musl linker}
: "${LINK_SAVE:?}"
mkdir -p "$LINK_SAVE"
# rustc's "-u <symbol>" (the PGO runtime's __llvm_profile_runtime) reaches
# zig cc as two arguments, and zig cc reads the symbol as an input file:
# pass it to the linker as -Wl,-u,<symbol>. Response files are expanded first.
args=()
for a in "$@"; do
  if [[ $a == @* && -f ${a#@} ]]; then mapfile -t more < "${a#@}"; args+=("${more[@]}"); else args+=("$a"); fi
done
fixed=()
for ((i = 0; i < ${#args[@]}; i++)); do
  if [[ ${args[i]} == -u ]]; then fixed+=("-Wl,-u,${args[i+1]}"); i=$((i + 1)); else fixed+=("${args[i]}"); fi
done
rc=0
ZIG_VERBOSE_LINK=1 "$real" "${fixed[@]}" 2> "$LINK_SAVE/zig-link.stderr" || rc=$?
grep -v '^ld.lld ' "$LINK_SAVE/zig-link.stderr" >&2 || true
[[ $rc -eq 0 ]] || exit $rc
printf '%s\n' "$PWD" > "$LINK_SAVE/cwd"
python3 -I /Users/jb/agent-targets/peak-cuts/scripts/link_save.py "$LINK_SAVE"
