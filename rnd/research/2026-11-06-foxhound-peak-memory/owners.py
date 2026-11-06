"""mem-research: heap at peak by owner, from heaptrack_print's folded stacks
(--print-flamegraph with --flamegraph-cost-type peak).

usage: owners.py <folded> [n]
Prints: total; by the leaf-most Goodfetch frame (goodfetch_* crate, with its
module path to depth 3); by the leaf-most library crate (first non-std,
non-allocator crate from the leaf); and top owner pairs.
"""
import re
import sys
from collections import defaultdict

NOISE = {"core", "alloc", "std", "hashbrown", "impl", "dyn", "mut", "const", "for", "as",
         "heaptrack", "smallvec", "parking_lot", "lock_api"}


def segs(frame):
    # v0 demangling marks each crate with its disambiguator: `tokio[1a2b]::…`.
    return [m[1] for m in re.finditer(r"([A-Za-z_][A-Za-z0-9_]*)\[[0-9a-f]+\]", frame)]


def lib_of(frame):
    if re.match(r"^(sqlite3|pcache1|btree|vdbe|yy|allocateBtreePage|openDatabase)", frame):
        return "sqlite (C)"
    if "duckdb" in frame:
        return "duckdb (C++)"
    if re.match(r"^(ZSTD|HUF_|FSE_)", frame):
        return "zstd (C)"
    if re.match(r"^Brotli", frame):
        return "brotli (C)"
    if re.match(r"^(OPENSSL|SSL_|bssl|CRYPTO|EVP_|X509|ssl_)", frame):
        return "boringssl (C)"
    for s in segs(frame):
        if s in NOISE:
            continue
        if s == "goodfetch_engine" and "::heap::" in frame:
            continue
        return s
    return None


def gf_of(frame):
    frame = re.sub(r"\[[0-9a-f]+\]", "", frame)
    m = re.search(r"(goodfetch_[a-z0-9_]+(?:::[a-z_][a-z0-9_]*){0,2})", frame)
    if not m or m[1].startswith("goodfetch_engine::heap"):
        return None
    return m[1]


import subprocess

rows = []
frames = set()
for line in open(sys.argv[1]):
    stack, _, n = line.rstrip("\n").rpartition(" ")
    try:
        n = int(n)
    except ValueError:
        continue
    if n <= 0:
        continue
    fs = [f.split(" (")[0] for f in stack.split(";")]
    frames.update(fs)
    rows.append((fs, n))
order = sorted(frames)
out = subprocess.run(["c++filt"], input="\n".join(order) + "\n", capture_output=True, text=True, check=True).stdout.splitlines()
DEM = dict(zip(order, out))

total = 0
by_gf, by_lib, pair = defaultdict(int), defaultdict(int), defaultdict(int)
for fs, n in rows:
    total += n
    gf = lib = None
    for f in reversed([DEM.get(x, x) for x in fs]):
        if lib is None:
            lb = lib_of(f)
            if lb:
                lib = "(own code)" if lb.startswith("goodfetch_") else lb
        if gf is None:
            gf = gf_of(f)
        if gf and lib:
            break
    by_gf[gf or "(no goodfetch frame)"] += n
    by_lib[lib or "(unknown)"] += n
    pair[(gf or "-", lib or "-")] += n
lim = int(sys.argv[2]) if len(sys.argv) > 2 else 30
MB = 1024 * 1024
print(f"total at peak {total/MB:.2f} MiB")
for title, d in (("by leaf-most Goodfetch frame", by_gf), ("by allocating library", by_lib), ("pairs", pair)):
    print(f"## {title}")
    for k, v in sorted(d.items(), key=lambda kv: -kv[1])[:lim]:
        if v >= 32 * 1024:
            print(f"{v/MB:8.2f}  {k}")
