"""peak-gap: attributes the executable's resident pages at a sampled peak
(sampler's <prefix>.pagemap: present bits per exe mapping) to the input
sections of lld's link map (-Map), split by origin: Rust (rlib / rustc
objects), each C or C++ archive, and Zig's runtime; and for read-only data,
named sections (orderable) against anonymous or merged ones. A page touched
counts once, split among the sections it holds by their bytes in it.

usage: pageown.py <exe> <map> <prefix> [top-n]
"""
import collections
import re
import subprocess
import sys

exe, mapf, prefix = sys.argv[1:4]
top = int(sys.argv[4]) if len(sys.argv) > 4 else 0
PAGE = 4096

# LOAD segments: (offset, vaddr, filesz, flags)
segs = []
for line in subprocess.run(["readelf", "-lW", exe], capture_output=True, text=True).stdout.splitlines():
    f = line.split()
    if f and f[0] == "LOAD":
        segs.append((int(f[1], 16), int(f[2], 16), int(f[4], 16), " ".join(f[6:-1])))

# present pages -> vaddr page set
present = set()
for line in open(prefix + ".pagemap"):
    off_hex, perms, bits = line.split()
    off = int(off_hex, 16)
    for i, b in enumerate(bits):
        if b != "1":
            continue
        fo = off + i * PAGE
        for so, va, fs, fl in segs:
            base = so & ~(PAGE - 1)
            if base <= fo < so + fs:
                present.add((fo - base + (va & ~(PAGE - 1))) // PAGE)
                break


def origin(path):
    a, sep, mem = path.partition("(")
    name = re.sub(r"^\d{4}-", "", a.rsplit("/", 1)[-1]) + (sep + mem.rsplit("/", 1)[-1] if sep else "")
    if name.startswith("<internal>"):
        return "linker"
    m = re.match(r"^(.+?)\((.+)\)$", name)
    arch, member = (m[1], m[2]) if m else (name, "")
    if member.endswith(".rcgu.o") or (not member and name.endswith(".rcgu.o")):
        return "rust"
    if arch.endswith(".rlib"):
        c = re.match(r"lib(.+?)-[0-9a-f]+\.rlib", arch)
        return "C:" + (c[1] if c else arch)
    if arch.endswith(".a"):
        return "zig:" + re.sub(r"^lib", "", arch[:-2])
    return "obj:" + name


# input sections from the map
secs = []  # (lo, hi, out, origin, section name)
out = None
for line in open(mapf):
    m = re.match(r"^\s*([0-9a-f]+)\s+[0-9a-f]+\s+([0-9a-f]+)\s+\d+ (.*)$", line)
    if not m:
        continue
    va, size, rest = int(m[1], 16), int(m[2], 16), m[3]
    lead = len(rest) - len(rest.lstrip())
    rest_s = rest.strip()
    if lead == 0:
        out = rest_s
        continue
    if lead == 8 and ":(" in rest_s and size:
        path, _, sec = rest_s.rpartition(":(")
        secs.append((va, va + size, out, origin(path), sec.rstrip(")")))

by = collections.Counter()
detail = collections.Counter()
for lo, hi, outs, org, sec in secs:
    for pg in range(lo // PAGE, (hi - 1) // PAGE + 1):
        if pg in present:
            a, b = max(lo, pg * PAGE), min(hi, (pg + 1) * PAGE)
            by[(outs, org)] += b - a
            if outs in (".rodata", ".data.rel.ro", ".eh_frame", ".gcc_except_table"):
                kind = "anon" if ".Lanon" in sec or "..L" in sec else ("merged" if re.search(r"\.(str|cst)\d", sec) else "named")
                detail[(outs, org.split(":")[0], kind)] += b - a

tot = collections.Counter()
for (o, g), v in by.items():
    tot[o] += v
print(f"== {prefix.rsplit('/', 1)[-1]}: {len(present) * PAGE / 2**20:.2f} MiB of the executable resident")
for o, v in tot.most_common():
    if v < 64 * 1024:
        continue
    parts = sorted(((g, x) for (oo, g), x in by.items() if oo == o), key=lambda t: -t[1])
    print(f"  {o:20s} {v / 2**20:6.2f} MiB: " + ", ".join(f"{g} {x / 2**20:.2f}" for g, x in parts[:6]))
print("  read-only data by kind:")
for k, v in sorted(detail.items(), key=lambda kv: -kv[1]):
    if v >= 32 * 1024:
        print(f"    {k[0]:18s} {k[1]:6s} {k[2]:7s} {v / 2**20:6.2f} MiB")
