#!/usr/bin/env python3
"""#2200 cap bisection: per run, Foxhound's startup (time to the first request) pooled over
raw and the sizes cells, wall and peak per cell, each against a reference run.

Startup: median, mean, share of trials over 0.6 s, and a two-sided Mann-Whitney U p by the
normal approximation with tie correction (pooled n = 60 a side). Per cell: median wall,
median peak RSS (MiB) and the change against the reference.
Usage: caps.py REF RUN [RUN...]
"""
import json, math, os, statistics as st, sys
from collections import defaultdict

T = os.environ.get("GF_COMPARE", "target/compare") + "/{}/trials.json"
MiB = 2**20


def load(name):
    d = json.load(open(T.format(name)))
    cells = defaultdict(list)
    for t in d["trials"]:
        if t["tool"] == "foxhound":
            cells[t["scenario"]].append(t)
    for t in d["sizes"].get("trials_run", []):
        if t["tool"] == "foxhound":
            cells[f"{t['site_pages']}p@{t['concurrency']}"].append(t)
    return cells


def mw_normal(a, b):
    xs = sorted([(v, 0) for v in a] + [(v, 1) for v in b])
    n1, n2, N = len(a), len(b), len(a) + len(b)
    r, i, ties = [0.0] * N, 0, 0.0
    while i < N:
        j = i
        while j + 1 < N and xs[j + 1][0] == xs[i][0]:
            j += 1
        for k in range(i, j + 1):
            r[k] = (i + j) / 2 + 1
        t = j - i + 1; ties += t**3 - t; i = j + 1
    r1 = sum(r[k] for k in range(N) if xs[k][1] == 0)
    u = r1 - n1 * (n1 + 1) / 2
    sd = math.sqrt(n1 * n2 / 12 * ((N + 1) - ties / (N * (N - 1))))
    z = (u - n1 * n2 / 2) / sd
    return z, math.erfc(abs(z) / math.sqrt(2))


def startup(cells):
    return [t["startup_seconds"] for ts in cells.values() for t in ts if t.get("startup_seconds") is not None]


ref = sys.argv[1]
R = load(ref)
rs = startup(R)
print(f"reference {ref}: startup n {len(rs)} median {st.median(rs):.3f} mean {st.mean(rs):.3f} "
      f"over 0.6 s {sum(v > 0.6 for v in rs)} max {max(rs):.2f} start loads max "
      f"{max(t['load'] for ts in R.values() for t in ts)}")
for name in sys.argv[2:]:
    C = load(name)
    cs = startup(C)
    z, p = mw_normal(cs, rs)
    print(f"\n{name}: startup n {len(cs)} median {st.median(cs):.3f} mean {st.mean(cs):.3f} over 0.6 s "
          f"{sum(v > 0.6 for v in cs)} max {max(cs):.2f}; against {ref}: z {z:+.2f} p {p:.2g}; "
          f"start loads max {max(t['load'] for ts in C.values() for t in ts)}")
    print("| cell | wall s | ref wall s | change | peak MiB | ref peak MiB | change |")
    for cell in sorted(C, key=lambda c: (c != "raw", c)):
        w = st.median(t["seconds"] for t in C[cell]); rw = st.median(t["seconds"] for t in R[cell])
        pk = st.median(t["peak_bytes"] for t in C[cell]) / MiB; rp = st.median(t["peak_bytes"] for t in R[cell]) / MiB
        print(f"| {cell} | {w:.3f} | {rw:.3f} | {100 * (w / rw - 1):+.1f}% | {pk:.2f} | {rp:.2f} | {pk - rp:+.2f} |")
