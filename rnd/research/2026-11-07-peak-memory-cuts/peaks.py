"""peak-cuts: phase A table. For each configuration and page count: the
kernel peak (the trial's peak_bytes) of each trial, its median, and the parts
at the sampled peak (smaps): the executable's pages, mimalloc's arena, other
anonymous memory, AnonHugePages, and THP_enabled from status.

usage: peaks.py [runs dir]
"""
import collections
import glob
import json
import os
import re
import statistics
import sys

R = sys.argv[1] if len(sys.argv) > 1 else "/Users/jb/agent-targets/peak-cuts/runs"
MiB = 1024


def parts(prefix):
    exe = arena = anon = huge = other = 0
    cur = None
    for line in open(prefix + ".smaps"):
        m = re.match(r"^[0-9a-f]+-[0-9a-f]+ \S+ \S+ \S+ \S+\s*(.*)$", line)
        if m:
            cur = m[1].strip()
            continue
        k = re.match(r"^(\w+):\s+(\d+) kB", line)
        if not k:
            continue
        if k[1] == "AnonHugePages":
            huge += int(k[2])
        if k[1] != "Rss":
            continue
        v = int(k[2])
        if cur.endswith(("fh-base", "spider", "fh-base-copy")) or "/bin/fh-" in cur or cur.endswith("goodfetch-foxhound"):
            exe += v
        elif cur == "[anon:mimalloc]":
            arena += v
        elif cur == "" or cur.startswith("[anon") or cur in ("[heap]", "[stack]"):
            anon += v
        else:
            other += v
    st = {}
    for line in open(prefix + ".status"):
        k, _, v = line.partition(":")
        st[k] = v.strip()
    return dict(exe=exe / MiB, arena=arena / MiB, anon=anon / MiB, other=other / MiB,
                huge=huge / MiB, thp=st.get("THP_enabled", "?"),
                rss_anon=int(st.get("RssAnon", "0 kB").split()[0]) / MiB,
                rss_file=int(st.get("RssFile", "0 kB").split()[0]) / MiB,
                threads=st.get("Threads", "?"))


rows = collections.defaultdict(list)
for js in glob.glob(os.path.join(R, "*-raw-*-c16.json-sample")):
    prefix = js[: -len(".json-sample")]
    name = os.path.basename(prefix)
    m = re.match(r"^(.+)-r(\d+)-raw-(\d+)-c16$", name)
    if not m:
        continue
    cfg, rep, n = m[1], int(m[2]), int(m[3])
    try:
        d = json.load(open(js))
    except ValueError:
        print(f"# {name}: no JSON", file=sys.stderr)
        continue
    p = parts(prefix) if os.path.exists(prefix + ".smaps") else {}
    rows[(cfg, n)].append((rep, d["peak_bytes"] / 2**20, d.get("seconds"), p))

print(f"{'config':8s} {'pages':>6s} {'peaks MiB':24s} {'median':>7s} {'exe':>5s} {'arena':>6s} {'anon':>5s} {'huge':>5s} {'RssAnon':>7s} {'RssFile':>7s} thp thr  seconds")
for (cfg, n) in sorted(rows, key=lambda k: (k[0], k[1])):
    v = sorted(rows[(cfg, n)])
    peaks = [x[1] for x in v]
    med = statistics.median(peaks)
    # parts of the trial whose peak is the median one
    rep = min(v, key=lambda x: abs(x[1] - med))
    p = rep[3]
    secs = " ".join(f"{x[2]:.2f}" for x in v if x[2] is not None)
    print(f"{cfg:8s} {n:6d} {' '.join(f'{x:6.1f}' for x in peaks):24s} {med:7.1f} "
          f"{p.get('exe', 0):5.1f} {p.get('arena', 0):6.1f} {p.get('anon', 0):5.1f} {p.get('huge', 0):5.1f} "
          f"{p.get('rss_anon', 0):7.1f} {p.get('rss_file', 0):7.1f} {p.get('thp', '?'):>3s} {p.get('threads', '?'):>3s}  {secs}")

print("\n# idle (3 s after start, before any request): sampled peak")
for f in sorted(glob.glob(os.path.join(R, "idle-*.smaps")) + glob.glob(os.path.join(R, "cache-*.smaps"))):
    prefix = f[:-6]
    p = parts(prefix)
    tot = p["exe"] + p["arena"] + p["anon"] + p["other"]
    print(f"{os.path.basename(prefix):30s} rss {tot:6.1f} exe {p['exe']:5.1f} arena {p['arena']:5.1f} anon {p['anon']:5.1f} huge {p['huge']:5.1f} thp {p['thp']}")
