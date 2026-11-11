"""peak-cuts: a linker symbol-ordering file from the PGO profile: every
function the training crawl ran (any of its counters above 0: LLVM's IR-level
profiles list "Block counts", not a function count), hottest (largest
counter) first, by its symbol name (a local function's profile name is
"<file>;<symbol>"). Only the Rust code is instrumented, so C and C++
functions (SQLite, mimalloc, BoringSSL, zstd, DuckDB) are not listed.

usage: orderfile.py <llvm-profdata show --all-functions --counts output> <out>
"""
import re
import sys

entries, name = [], None
for line in open(sys.argv[1]):
    m = re.match(r"^  (\S.*):$", line)
    if m:
        name = m[1]
        continue
    m = re.match(r"^    Block counts: \[(.*)\]$", line)
    if m and name is not None:
        counts = [int(x) for x in m[1].split(",") if x.strip()]
        entries.append((max(counts, default=0), name.split(";")[-1]))
        name = None
total = len(entries)
hot = sorted((e for e in entries if e[0] > 0), key=lambda e: -e[0])
seen, out = set(), []
for _, n in hot:
    if n not in seen:
        seen.add(n)
        out.append(n)
open(sys.argv[2], "w").write("\n".join(out) + "\n")
print(f"orderfile: {len(out)} of {total} instrumented functions entered", file=sys.stderr)
