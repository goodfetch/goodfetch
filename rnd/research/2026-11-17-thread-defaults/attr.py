"""peak-gap: attributes a process's resident memory at its sampled peak
(sampler2.py's <prefix>.smaps and .tasks) to owners: the executable, mimalloc's
arenas, musl's brk heap, each thread's stack (the mapping holding its stack
pointer, named by the thread's comm), guarded mappings no thread points into,
and every other anonymous mapping (listed with its size and permissions).

usage: attr.py <prefix> [<prefix>...]   (one table per prefix)
"""
import collections
import re
import sys


def maps(prefix):
    out, cur = [], None
    for line in open(prefix + ".smaps"):
        m = re.match(r"^([0-9a-f]+)-([0-9a-f]+) (\S+) \S+ \S+ \S+\s*(.*)$", line)
        if m:
            cur = dict(lo=int(m[1], 16), hi=int(m[2], 16), perm=m[3], name=m[4].strip(), rss=0, anon=0)
            out.append(cur)
            continue
        k = re.match(r"^(\w+):\s+(\d+) kB", line)
        if k and k[1] == "Rss":
            cur["rss"] = int(k[2])
        elif k and k[1] == "Anonymous":
            cur["anon"] = int(k[2])
    return out


def tasks(prefix):
    rows = []
    for line in open(prefix + ".tasks"):
        tid, comm, sp, pc = line.split()
        rows.append((tid, comm, int(sp, 16) if sp.startswith("0x") else None))
    return rows


for prefix in sys.argv[1:]:
    ms, ts = maps(prefix), tasks(prefix)
    owner = collections.Counter()
    count = collections.Counter()
    others = []
    for i, m in enumerate(ms):
        if m["rss"] == 0:
            continue
        holders = [c for _, c, sp in ts if sp is not None and m["lo"] <= sp < m["hi"]]
        guarded = i > 0 and ms[i - 1]["perm"].startswith("---") and ms[i - 1]["hi"] == m["lo"]
        n = m["name"]
        if "/bin/fh-" in n or n.endswith(("spider", "goodfetch-foxhound")):
            key = "exe " + m["perm"]
        elif n.startswith("[anon:mimalloc") or "mimalloc" in n:
            key = "mimalloc arena"
        elif n == "[heap]":
            key = "brk heap (musl malloc)"
        elif n == "[stack]":
            key = "stack main"
        elif holders:
            key = "stack " + re.sub(r"[-_]?\d+$", "", holders[0])
        elif n in ("[vdso]", "[vvar]", "[vvar_vclock]"):
            key = "vdso/vvar"
        elif n and not n.startswith("["):
            key = "file " + n
        elif guarded:
            key = "guarded, no thread sp"
            others.append(m | dict(kind="guarded"))
        else:
            key = "other anon " + m["perm"]
            others.append(m | dict(kind="other"))
        owner[key] += m["rss"]
        count[key] += 1
    tot = sum(owner.values())
    print(f"== {prefix.rsplit('/', 1)[-1]}: {tot / 1024:.1f} MiB resident, {len(ts)} threads")
    for k, v in sorted(owner.items(), key=lambda kv: -kv[1]):
        print(f"  {v / 1024:7.2f} MiB  {count[k]:3d}  {k}")
    print("  threads:", " ".join(f"{c}x{n}" for c, n in collections.Counter(re.sub(r'[-_]?\d+$', '', c) for _, c, _ in ts).most_common()))
    for m in sorted(others, key=lambda m: -m["rss"])[:15]:
        print(f"    {m['kind']:8s} {m['lo']:x} size {(m['hi'] - m['lo']) // 1024:7d} kB rss {m['rss']:6d} kB {m['perm']} {m['name']}")
