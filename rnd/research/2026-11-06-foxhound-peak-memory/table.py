"""mem-research: one line per sampled run: peak RSS and its parts."""
import glob
import os
import sys

sys.path.insert(0, "/Users/jb/agent-targets/mem-research/scripts")
from smaps_sum import parse, group  # noqa: E402

os.chdir("/Users/jb/agent-targets/mem-research/runs")
names = sys.argv[1:] or sorted(p[:-6] for p in glob.glob("*.smaps"))
for f in names:
    exe = "spider" if f.startswith("sp") else ("fh-system" if f.startswith("fs") or f.endswith("system") else "fh-mimalloc")
    g = {}
    for m in parse(f + ".smaps"):
        k = group(m, exe)
        g[k] = g.get(k, 0) + m["f"].get("Rss", 0)
    tot = sum(g.values())
    exe_rss = sum(v for k, v in g.items() if k.startswith("exe"))
    heap = g.get("[anon:mimalloc]", 0) + g.get("[heap]", 0)
    anon = g.get("anon", 0)
    libs = sum(v for k, v in g.items() if k.startswith("lib"))
    hwm = ([l.split()[1] for l in open(f + ".status") if l.startswith("VmHWM")] or ["0"])[0]
    thr = open(f + ".threads").read().strip()
    js = ""
    if os.path.exists(f + ".json-sample"):
        import json
        try:
            d = json.load(open(f + ".json-sample"))
            js = f"rusage-peak {d['peak_bytes']/2**20:6.1f} s {d['seconds']:.2f}"
        except ValueError:
            js = "(no JSON)"
    print(f"{f:22s} peak {tot/1024:6.1f} hwm {int(hwm)/1024:6.1f} exe {exe_rss/1024:5.1f} "
          f"heap {heap/1024:6.1f} anon {anon/1024:5.1f} libs {libs/1024:4.1f} thr {thr:>3s} {js}")
