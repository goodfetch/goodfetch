#!/usr/bin/env python3
"""#2200: defaults (A) against another run (B), Foxhound's cells, from trials.json (#2742).

Per cell: median, min-max and n of wall time, median CPU (user + system), peak RSS and
peak threads, the largest start load, each run's work check; the change of B's median
against A's and a two-sided Mann-Whitney U p (exact, by enumeration of ranks' sums for
n <= 10 each, ties averaged) so "beyond trial noise" is a stated test, not a reading.
Usage: compare2.py A B
"""
import itertools, json, os, statistics, sys
from collections import defaultdict

T = os.environ.get("GF_COMPARE", "target/compare") + "/{}/trials.json"
MiB = 2**20
A, B = sys.argv[1], sys.argv[2]


def load(name):
    d = json.load(open(T.format(name)))
    cells = defaultdict(list)
    for t in d["trials"]:
        if t["tool"] == "foxhound":
            cells[(t["scenario"], t["cell"])].append(t)
    for t in d["sizes"].get("trials_run", []):
        if t["tool"] == "foxhound":
            cells[("sizes", f"{t['site_pages']}p@{t['concurrency']}")].append(t)
    auto = [e["judged"] for e in d.get("auto", []) if e["tool"] == "foxhound"]
    rules = defaultdict(list)
    for e in d.get("rules", []):
        if e["tool"] == "foxhound":
            for task in e["tasks"]:
                rules[(e["scenario"], task["task"])].append(
                    (task["requests"], task["flagged"], task["refused"], task["wanted_fetched"], task["wanted"],
                     task["over_in_flight_cap"], task["retry_after_ignored"]))
    instr = [r["count"] for r in d["instructions"].get("runs", []) if r["tool"] == "foxhound"]
    return d, cells, auto, rules, instr


def ranks(xs):
    s = sorted(range(len(xs)), key=lambda i: xs[i])
    r = [0.0] * len(xs)
    i = 0
    while i < len(s):
        j = i
        while j + 1 < len(s) and xs[s[j + 1]] == xs[s[i]]:
            j += 1
        for k in range(i, j + 1):
            r[s[k]] = (i + j) / 2 + 1
        i = j + 1
    return r


def mann_whitney_p(a, b):
    xs = a + b
    r = ranks(xs)
    n = len(a)
    observed = sum(r[:n])
    mean = n * (len(xs) + 1) / 2
    dev = abs(observed - mean)
    total = hits = 0
    for combo in itertools.combinations(range(len(xs)), n):
        total += 1
        if abs(sum(r[i] for i in combo) - mean) >= dev - 1e-9:
            hits += 1
    return hits / total


def summary(ts):
    sec = [t["seconds"] for t in ts]
    return dict(n=len(ts), sec=sec, med=statistics.median(sec), lo=min(sec), hi=max(sec),
                cpu=statistics.median([t["user_seconds"] + t["system_seconds"] for t in ts]),
                peak=statistics.median([t["peak_bytes"] for t in ts]) / MiB,
                peak_lo=min(t["peak_bytes"] for t in ts) / MiB, peak_hi=max(t["peak_bytes"] for t in ts) / MiB,
                peaks=[t["peak_bytes"] / MiB for t in ts],
                thr=statistics.median([t["sampled"]["peak_threads"] for t in ts]),
                load=max(t["load"] for t in ts),
                ok=all(not t["work"]["missing"] and not t["work"]["extra"] for t in ts))


da, ca, aa, ra, ia = load(A)
db, cb, ab, rb, ib = load(B)
for name, d in ((A, da), (B, db)):
    print(f"# {name}: commit {d['commit'][:9]}, {d['machine']['name']}, {d['machine']['cpu']}, "
          f"{d['machine']['threads']} threads, load at start {d['machine']['load']}, seed {d.get('order_seed')}")
print()
print(f"| Cell | {A}: median s [min-max] n | {B}: median s [min-max] n | wall Δ | p | CPU s {A} → {B} | peak MiB {A} → {B} (p) | threads | max start load |")
print("|---|---|---|---|---|---|---|---|---|")
for key in sorted(set(ca) | set(cb)):
    a, b = ca.get(key), cb.get(key)
    if not a or not b:
        print(f"| {key[0]}/{key[1]} | {'-' if not a else len(a)} | {'-' if not b else len(b)} | | | | | | |")
        continue
    sa, sb = summary(a), summary(b)
    p = mann_whitney_p(sa["sec"], sb["sec"])
    pp = mann_whitney_p(sa["peaks"], sb["peaks"])
    flag = "" if sa["ok"] and sb["ok"] else " WORK!"
    print(f"| {key[0]}/{key[1]} | {sa['med']:.3f} [{sa['lo']:.3f}-{sa['hi']:.3f}] {sa['n']} | "
          f"{sb['med']:.3f} [{sb['lo']:.3f}-{sb['hi']:.3f}] {sb['n']} | {100 * (sb['med'] / sa['med'] - 1):+.1f}% | {p:.3f} | "
          f"{sa['cpu']:.2f} → {sb['cpu']:.2f} | {sa['peak']:.1f} → {sb['peak']:.1f} ({pp:.3f}) | "
          f"{sa['thr']:.0f} → {sb['thr']:.0f} | {sa['load']} / {sb['load']}{flag} |")
print()
for name, auto in ((A, aa), (B, ab)):
    if auto:
        s = [j["seconds"] for j in auto]
        print(f"auto {name}: n={len(s)} median {statistics.median(s):.3f} s [{min(s):.3f}-{max(s):.3f}], "
              f"pages {[j['pages'] for j in auto]}, rendered {[j['rendered'] for j in auto]}, wrong {[j['wrong'] for j in auto]}")
for key in sorted(set(ra) | set(rb)):
    print(f"rules {key[0]} {key[1]} (requests, flagged, refused, wanted fetched/wanted, over cap, retry-after ignored): "
          f"{A} {sorted(set(ra.get(key, [])))} | {B} {sorted(set(rb.get(key, [])))}")
print(f"instructions {A}: {ia}")
print(f"instructions {B}: {ib}")
