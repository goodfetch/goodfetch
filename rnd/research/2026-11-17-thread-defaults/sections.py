#!/usr/bin/env python3
"""#2200 (e): the executable's resident pages at a sampled peak, by ELF output section.

pageown.py's split by input section needs LLD's link map (-Map), which compare's
release build does not write; this keeps its first step (present pages of the
exe's mappings -> virtual pages through the LOAD segments) and names each page's
bytes by the output section (readelf -S) that holds them.

usage: sections.py <exe> <prefix> [<prefix>...]
"""
import collections, subprocess, sys

PAGE = 4096
exe = sys.argv[1]
segs = []
for line in subprocess.run(["readelf", "-lW", exe], capture_output=True, text=True).stdout.splitlines():
    f = line.split()
    if f and f[0] == "LOAD":
        segs.append((int(f[1], 16), int(f[2], 16), int(f[4], 16)))
secs = []
for line in subprocess.run(["readelf", "-SW", exe], capture_output=True, text=True).stdout.splitlines():
    f = line.replace("[ ", "[").split()
    if len(f) > 6 and f[0].startswith("[") and f[0].endswith("]") and f[1].startswith("."):
        addr, size = int(f[3], 16), int(f[5], 16)
        if addr and size and f[2] != "NOBITS":
            secs.append((addr, addr + size, f[1]))

for prefix in sys.argv[2:]:
    present = set()
    for line in open(prefix + ".pagemap"):
        off_hex, _perms, bits = line.split()
        off = int(off_hex, 16)
        for i, b in enumerate(bits):
            if b != "1":
                continue
            fo = off + i * PAGE
            for so, va, fs in segs:
                base = so & ~(PAGE - 1)
                if base <= fo < so + fs:
                    present.add((fo - base + (va & ~(PAGE - 1))) // PAGE)
                    break
    by = collections.Counter()
    for lo, hi, name in secs:
        for pg in range(lo // PAGE, (hi - 1) // PAGE + 1):
            if pg in present:
                by[name] += min(hi, (pg + 1) * PAGE) - max(lo, pg * PAGE)
    print(f"== {prefix.rsplit('/', 1)[-1]}: {len(present) * PAGE / 2**20:.2f} MiB of the executable resident")
    for name, v in by.most_common():
        if v >= 16 * 1024:
            print(f"  {name:22s} {v / 2**20:6.2f} MiB")
