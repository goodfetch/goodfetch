"""peak-cuts: samples one process until it exits.

usage: sampler.py <pid> <out-prefix> <exe-basename>

Every 10 ms reads /proc/<pid>/smaps_rollup; appends "t_ms rss_kB anon_kB" to
<out>.series; at each new RSS peak saves smaps, status, the thread count and
the exe mappings' present pages from /proc/<pid>/pagemap (bit 63), as
"<file offset hex> <present 0/1 string>" lines per mapping, to <out>.pagemap.
"""
import os
import sys
import time

pid = exe = None
PAGE = os.sysconf("SC_PAGE_SIZE")


def rollup():
    rss = anon = None
    with open(f"/proc/{pid}/smaps_rollup") as f:
        for line in f:
            if line.startswith("Rss:"):
                rss = int(line.split()[1])
            elif line.startswith("Anonymous:"):
                anon = int(line.split()[1])
    return rss, anon


def pagemap():
    lines = []
    with open(f"/proc/{pid}/maps") as maps, open(f"/proc/{pid}/pagemap", "rb") as pm:
        for line in maps:
            parts = line.split()
            if len(parts) < 6 or not parts[5].endswith(exe):
                continue
            start, end = (int(x, 16) for x in parts[0].split("-"))
            off = int(parts[2], 16)
            n = (end - start) // PAGE
            pm.seek(start // PAGE * 8)
            raw = pm.read(n * 8)
            bits = "".join(
                "1" if raw[i * 8 + 7] & 0x80 else "0" for i in range(len(raw) // 8)
            )
            lines.append(f"{off:x} {parts[1]} {bits}")
    return "\n".join(lines) + "\n"


def copy(src, dst):
    with open(src) as f:
        data = f.read()
    with open(dst, "w") as f:
        f.write(data)

