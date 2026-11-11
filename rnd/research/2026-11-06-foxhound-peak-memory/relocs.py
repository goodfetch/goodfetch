"""mem-research: the executable's dynamic relocations, by target section and
by the crate of the symbol whose bytes they patch (binattr's attribution).

usage: relocs.py <exe>
"""
import bisect
import re
import subprocess
import sys

sys.argv = [sys.argv[0], sys.argv[1]]
import binattr  # noqa: E402  (runs its static pass; output discarded below)

exe = sys.argv[1]
out = subprocess.run(["readelf", "-rW", exe], check=True, capture_output=True, text=True).stdout
by_sec, by_crate, n = {}, {}, 0
for line in out.splitlines():
    m = re.match(r"^([0-9a-f]{16})\s+[0-9a-f]+\s+(\S+)", line)
    if not m:
        continue
    off = int(m[1], 16)
    n += 1
    sec, _ = binattr.section_of(off)
    by_sec[sec] = by_sec.get(sec, 0) + 1
    i = bisect.bisect_right(binattr.starts, off) - 1
    c = "(no symbol)"
    if i >= 0:
        a, s, name = binattr.syms[i]
        if a <= off < a + s:
            c = binattr.crate_of(name)
    by_crate[c] = by_crate.get(c, 0) + 1
print(f"relocations: {n} ({n * 24 / 2**20:.2f} MiB of .rela.dyn)")
for k, v in sorted(by_sec.items(), key=lambda kv: -kv[1])[:8]:
    print(f"  section {k}: {v} ({v * 8 / 2**20:.2f} MiB of patched pointers)")
for k, v in sorted(by_crate.items(), key=lambda kv: -kv[1])[:12]:
    print(f"  crate {k}: {v}")
