"""peak-cuts: phase B. Wall time and pages per second of scenario 1 with
mimalloc's huge pages on and off, from interleaved pairs: per page count, the
median of each, the median of the paired differences (off - on) with a 95%
bootstrap interval, and a sign test.

usage: wall.py <tag> [runs/wall dir]
"""
import glob
import json
import math
import os
import random
import re
import statistics
import sys

tag = sys.argv[1]
R = sys.argv[2] if len(sys.argv) > 2 else "/Users/jb/agent-targets/peak-cuts/runs/wall"
data = {}
for f in glob.glob(os.path.join(R, f"{tag}-*.json")):
    m = re.match(rf"^{re.escape(tag)}-(on|off)-(\d+)-(\d+)\.json$", os.path.basename(f))
    if not m:
        continue
    try:
        d = json.load(open(f))
    except ValueError:
        continue
    data[(m[1], int(m[2]), int(m[3]))] = d
for n in sorted({k[1] for k in data}):
    pairs = sorted(i for (l, nn, i) in data if l == "on" and nn == n and ("off", n, i) in data)
    on = [data[("on", n, i)]["seconds"] for i in pairs]
    off = [data[("off", n, i)]["seconds"] for i in pairs]
    diff = [b - a for a, b in zip(on, off)]
    random.seed(2162)
    boots = sorted(statistics.median(random.choices(diff, k=len(diff))) for _ in range(10000))
    lo, hi = boots[249], boots[9749]
    pos = sum(d > 0 for d in diff); neg = sum(d < 0 for d in diff); k = pos + neg
    p = min(1.0, 2 * sum(math.comb(k, j) for j in range(0, min(pos, neg) + 1)) / 2**k) if k else 1.0
    cpu = lambda d: d["user_seconds"] + d["system_seconds"]
    con = [cpu(data[("on", n, i)]) for i in pairs]
    coff = [cpu(data[("off", n, i)]) for i in pairs]
    cdiff = [b - a for a, b in zip(con, coff)]
    cb = sorted(statistics.median(random.choices(cdiff, k=len(cdiff))) for _ in range(10000))
    print(f"{tag} {n} pages CPU s: on median {statistics.median(con):.2f}, off {statistics.median(coff):.2f}; "
          f"paired off-on median {statistics.median(cdiff):+.2f} s [95% {cb[249]:+.2f}, {cb[9749]:+.2f}] "
          f"({statistics.median(cdiff)/statistics.median(con)*100:+.1f}%)")
    pps_on, pps_off = n / statistics.median(on), n / statistics.median(off)
    print(f"{tag} {n} pages, {len(pairs)} pairs: on median {statistics.median(on):.3f} s "
          f"(IQR {statistics.quantiles(on, n=4)[0]:.3f}-{statistics.quantiles(on, n=4)[2]:.3f}), "
          f"off median {statistics.median(off):.3f} s "
          f"(IQR {statistics.quantiles(off, n=4)[0]:.3f}-{statistics.quantiles(off, n=4)[2]:.3f}); "
          f"pages/s on {pps_on:.0f} off {pps_off:.0f}; paired off-on median {statistics.median(diff)*1000:+.1f} ms "
          f"[95% {lo*1000:+.1f}, {hi*1000:+.1f}] ({statistics.median(diff)/statistics.median(on)*100:+.1f}%); "
          f"off slower in {pos}/{k}, sign test p={p:.3f}")
