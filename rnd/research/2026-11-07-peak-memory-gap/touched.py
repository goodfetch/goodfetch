"""peak-gap: from an exact touch map (a trial run with fault-around at one page
and MADV_RANDOM, so every resident page of the executable was touched itself),
lists the input sections that lie on touched pages, from lld's -Map:
  <out>.text-syms   function symbols of touched .text sections (order-file input)
  <out>.data-syms   named symbols of touched .rodata/.data.rel.ro sections
  <out>.anon-secs   names of touched anonymous .rodata sections (.rodata..L*)
and prints totals: touched bytes per output section and origin.

usage: touched.py <exe> <map> <pagemap-prefix> <out>
"""
import collections
import re
import sys

sys.argv, (exe, mapf, prefix, out) = sys.argv[:1], sys.argv[1:5]
sys.argv += [exe, mapf, prefix]
src = open(__file__.replace("touched.py", "pageown.py")).read()
ns = {}
exec(compile(src.split("# input sections from the map")[0], "pageown", "exec"), ns)
present, origin, PAGE = ns["present"], ns["origin"], ns["PAGE"]

secs = []  # (lo, hi, out, origin, secname, [symbols])
cur_out = None
for line in open(mapf):
    m = re.match(r"^\s*([0-9a-f]+)\s+[0-9a-f]+\s+([0-9a-f]+)\s+\d+ (.*)$", line)
    if not m:
        continue
    va, size, rest = int(m[1], 16), int(m[2], 16), m[3]
    lead = len(rest) - len(rest.lstrip())
    r = rest.strip()
    if lead == 0:
        cur_out = r
    elif lead == 8 and ":(" in r:
        path, _, sec = r.rpartition(":(")
        secs.append([va, va + size, cur_out, origin(path), sec.rstrip(")"), []])
    elif lead == 16 and secs:
        secs[-1][5].append(r)

hot = collections.Counter()
tsyms, dsyms, anon = [], [], []
for lo, hi, o, g, sec, syms in secs:
    if hi <= lo:
        continue
    if not any(p in present for p in range(lo // PAGE, (hi - 1) // PAGE + 1)):
        continue
    hot[(o, g.split(":")[0])] += hi - lo
    if o == ".text":
        tsyms += syms
    elif o in (".rodata", ".data.rel.ro"):
        if "..L" in sec:
            anon.append(sec)
        elif not re.search(r"\.(str|cst)\d", sec):
            dsyms += syms
open(out + ".text-syms", "w").write("\n".join(tsyms) + "\n")
open(out + ".data-syms", "w").write("\n".join(dsyms) + "\n")
open(out + ".anon-secs", "w").write("\n".join(anon) + "\n")
print(f"touched pages: {len(present)} ({len(present) * PAGE / 2**20:.2f} MiB)")
for (o, g), v in sorted(hot.items(), key=lambda kv: -kv[1]):
    if v > 32 * 1024:
        print(f"  sections on touched pages: {o:14s} {g:6s} {v / 2**20:6.2f} MiB")
print(f"{len(tsyms)} text symbols, {len(dsyms)} data symbols, {len(anon)} anonymous sections")
