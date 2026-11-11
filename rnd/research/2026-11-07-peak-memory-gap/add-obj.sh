#!/usr/bin/env bash
# peak-gap: copies a saved link and inserts extra object files before its first
# archive, e.g. malloc-shim.o (the C code's malloc family to mimalloc).
# usage: add-obj.sh <save> <new save> <object...>
set -euo pipefail
src=$1 dst=$2; shift 2
mkdir -p "$dst"; cp "$src/cwd" "$dst/cwd"
python3 -I - "$src/args" "$dst/args" "$@" <<'PY'
import sys
lines = open(sys.argv[1]).read().splitlines()
i = next(i for i, l in enumerate(lines) if l.endswith(".a") or l.endswith(".rlib"))
lines[i:i] = sys.argv[3:]
open(sys.argv[2], "w").write("\n".join(lines) + "\n")
PY
diff "$src/args" "$dst/args" || true
