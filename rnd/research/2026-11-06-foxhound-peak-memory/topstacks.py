"""mem-research: the largest peak stacks of a folded file, demangled, with
std/core/alloc/tokio runtime frames dropped. usage: topstacks.py <folded> [n] [filter-crate]"""
import re
import subprocess
import sys

n = int(sys.argv[2]) if len(sys.argv) > 2 else 8
want = sys.argv[3] if len(sys.argv) > 3 else None
rows = []
for line in open(sys.argv[1]):
    stack, _, c = line.rstrip("\n").rpartition(" ")
    if c.isdigit() and int(c) > 0:
        rows.append((int(c), [f.split(" (")[0] for f in stack.split(";")]))
frames = sorted({f for _, fs in rows for f in fs})
dem = dict(zip(frames, subprocess.run(["c++filt"], input="\n".join(frames) + "\n", capture_output=True, text=True).stdout.splitlines()))
# merge by the cleaned stack's last 6 meaningful frames
agg = {}
for c, fs in rows:
    clean = []
    for f in fs:
        d = re.sub(r"\[[0-9a-f]+\]", "", dem.get(f, f))
        if re.match(r"^(<?(std|core|alloc|tokio|futures_util|tokio_util)::|__clone|start_thread|__rust)", d):
            continue
        clean.append(d[:110])
    key = tuple(clean[-6:])
    if want and not any(want in x for x in key):
        continue
    agg[key] = agg.get(key, 0) + c
for key, c in sorted(agg.items(), key=lambda kv: -kv[1])[:n]:
    print(f"{c/2**20:7.2f} MiB")
    for f in key:
        print(f"           {f}")
