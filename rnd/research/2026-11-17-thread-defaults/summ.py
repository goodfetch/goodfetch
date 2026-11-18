#!/usr/bin/env python3
"""#2200: medians with min-max, n and an exact two-sided Mann-Whitney U p, each setting against the first.

usage: summ.py daemon RESULTS.json|.jsonl
       summ.py rss REF.jsonl OTHER.jsonl...   (each one setting's `xtask rss measure --out` lines)
"""
import itertools, json, os, statistics as st, sys

MiB = 2**20


def ranks(xs):
    s = sorted(range(len(xs)), key=lambda i: xs[i]); r = [0.0] * len(xs); i = 0
    while i < len(s):
        j = i
        while j + 1 < len(s) and xs[s[j + 1]] == xs[s[i]]:
            j += 1
        for k in range(i, j + 1):
            r[s[k]] = (i + j) / 2 + 1
        i = j + 1
    return r


def mw(a, b):
    xs = a + b; r = ranks(xs); n = len(a); mean = n * (len(xs) + 1) / 2
    dev = abs(sum(r[:n]) - mean); tot = hit = 0
    for c in itertools.combinations(range(len(xs)), n):
        tot += 1
        hit += abs(sum(r[i] for i in c) - mean) >= dev - 1e-9
    return hit / tot


def fmt(v, f="{:.3f}"):
    return f"{f.format(st.median(v))} [{f.format(min(v))}-{f.format(max(v))}] {len(v)}"


def rows(path):
    text = open(path).read()
    return json.loads(text) if text.lstrip().startswith("[") else [json.loads(l) for l in text.splitlines() if l.startswith("{")]


def daemon(path):
    rs = rows(path)
    print("| Shape | Setting | Wall s median [min-max] n | CPU s | Peak RSS MiB | Peak threads | Start load max | All pages | wall Δ, p | peak p |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    settings = list(dict.fromkeys(r["setting"] for r in rs))
    for shape in sorted({(r["runs"], r["pages"]) for r in rs}):
        base = None
        for s in settings:
            x = [r for r in rs if (r["runs"], r["pages"]) == shape and r["setting"] == s]
            if not x:
                continue
            sec = [r["seconds"] for r in x]; peak = [r["peak_rss_bytes"] / MiB for r in x]
            ok = all(e == 0 for r in x for e in r["exits"]) and all(v == shape[1] for r in x for v in r["succeeded"])
            if base:
                cmp = f"{100 * (st.median(sec) / st.median(base[0]) - 1):+.1f}%, {mw(base[0], sec):.4f} | {mw(base[1], peak):.4f}"
            else:
                cmp, base = " | ", (sec, peak)
            print(f"| {shape[0]}x{shape[1]} | {s} | {fmt(sec)} | {st.median(r['cpu_seconds'] for r in x):.1f} | "
                  f"{fmt(peak, '{:.1f}')} | {st.median(r['peak_threads'] for r in x):.0f} | {max(r['load'] for r in x)} | {ok} | {cmp} |")


def rss(paths):
    sets = {os.path.basename(p).rsplit(".", 1)[0]: [json.loads(l) for l in open(p)] for p in paths}
    names = list(sets)
    print("| Scenario | " + " | ".join(f"{n} MiB" for n in names) + " | " + " | ".join(f"{n} − {names[0]}, p" for n in names[1:]) + " |")
    print("|---" * (2 * len(names)) + "|")
    for sc in sorted({r["scenario"] for rs in sets.values() for r in rs}, key=lambda s: int(s.split("-")[-1])):
        v = {n: [r["bytes"] / MiB for r in sets[n] if r["scenario"] == sc] for n in names}
        ref = v[names[0]]
        cells = [fmt(v[n], "{:.2f}") if v[n] else "-" for n in names]
        diffs = [f"{st.median(v[n]) - st.median(ref):+.2f}, {mw(ref, v[n]):.3f}" if v[n] and ref else "-" for n in names[1:]]
        print(f"| {sc} | " + " | ".join(cells) + " | " + " | ".join(diffs) + " |")


if __name__ == "__main__":
    daemon(sys.argv[2]) if sys.argv[1] == "daemon" else rss(sys.argv[2:])
