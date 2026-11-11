#!/usr/bin/env bash
# peak-cuts: derives a static-PIE link from a saved non-PIE one: rcrt1.o (musl's
# self-relocating start) in place of crt1.o, -pie in place of -no-pie, image
# base 0, and Zig's runtime archives (libc, libc++, libc++abi, libunwind,
# ubsan_rt, zigc, compiler_rt) as the same Zig builds them for a -fPIE link
# (zig c++ -target x86_64-linux-musl -fPIE, its cache paths given in pic.list).
# usage: pie-args.sh <save> <new save> <pic.list>
set -euo pipefail
src=$1 dst=$2 list=$3
mkdir -p "$dst"
cp "$src/cwd" "$dst/cwd"
cp "$src/args" "$dst/args"
sed -i -e 's|^-no-pie$|-pie|' -e 's|^--image-base=16777216$|--image-base=0|' "$dst/args"
python3 -I - "$dst/args" "$list" <<'PY'
import os, sys
args, lst = sys.argv[1], sys.argv[2]
lines = open(args).read().splitlines()
for pic in open(lst).read().split():
    base = os.path.basename(pic)
    base = "crt1.o" if base == "rcrt1.o" else base
    hits = [i for i, l in enumerate(lines) if os.path.basename(l).split("-", 1)[-1] == base and l.startswith("/")]
    assert len(hits) == 1, (base, hits)
    lines[hits[0]] = pic
open(args, "w").write("\n".join(lines) + "\n")
PY
diff "$src/args" "$dst/args" || true
