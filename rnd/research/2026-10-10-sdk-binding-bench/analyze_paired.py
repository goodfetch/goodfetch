"""Paired analysis: per-block ratio in-process / stdio RPC (time; <1 means in-process is faster)."""
import json, glob, os, random, statistics, sys

O = os.path.join(os.path.dirname(os.path.abspath(__file__)), "paired")
blocks = {}
for path in glob.glob(os.path.join(O, "b*_*.jsonl")):
    blk, runner = os.path.basename(path)[1:-6].split("_", 1)
    rows = blocks.setdefault(int(blk), {}).setdefault(runner, {})
    for line in open(path):
        line = line.strip()
        if not line.startswith("{"):
            continue
        d = json.loads(line)
        if d.get("meta") == "pre":
            rows["_pre"] = d
        elif "bench" in d:
            rows[d["bench"]] = d

def p50(d):
    return d.get("p50_ns", d.get("p50Ns"))

# (label, in-process runner, in-process bench, rpc runner, rpc bench, kind)
pairs = []
for size in (64, 100_000, 1_000_000):
    pairs.append((f"Node handler call, {size} B page", "napi", f"rust_calls_js_handler_seq_{size}", "stdio_node", f"node_stdio_seq_{size}", "p50"))
    pairs.append((f"Python handler call, {size} B page", "pyo3", f"rust_calls_py_handler_seq_{size}", "stdio_py", f"py_stdio_seq_{size}", "p50"))
for size in (64, 100_000):
    pairs.append((f"Node 16 concurrent handlers, {size} B page", "napi", f"rust_calls_js_handler_c16_{size}", "stdio_node", f"node_stdio_c16_{size}", "rps"))

def boot_ci(xs, n=10000, seed=7):
    rnd = random.Random(seed)
    meds = sorted(statistics.median(rnd.choices(xs, k=len(xs))) for _ in range(n))
    return meds[int(0.025 * n)], meds[int(0.975 * n)]

complete = sorted(b for b, r in blocks.items() if all(k in r for k in ("napi", "pyo3", "stdio_node", "stdio_py")))
print(f"complete blocks: {len(complete)} {complete}")
for b in complete:
    pre = {r: blocks[b][r].get("_pre", {}) for r in ("napi", "stdio_node", "pyo3", "stdio_py")}
    print(f"  block {b}: foreign CPU % " + ", ".join(f"{r}={pre[r].get('foreign_cpu')}" for r in pre))
print()
print("| Comparison | Blocks | In-process p50 (median) | RPC p50 (median) | Ratio in-process/RPC (median) | 95% CI |")
print("| --- | ---: | ---: | ---: | ---: | --- |")
for label, ri, bi, rr, br, kind in pairs:
    ratios, ins, rpcs = [], [], []
    for b in complete:
        a, c = blocks[b][ri].get(bi), blocks[b][rr].get(br)
        if not a or not c:
            continue
        if kind == "p50":
            x, y = p50(a), p50(c)
            ins.append(x / 1000); rpcs.append(y / 1000)
            ratios.append(x / y)
        else:
            x, y = a["calls_per_s"], c["calls_per_s"]
            ins.append(x); rpcs.append(y)
            ratios.append(y / x)  # time ratio = rpc_rps / inproc_rps
    if len(ratios) < 3:
        print(f"| {label} | {len(ratios)} | n/a | n/a | n/a | n/a |")
        continue
    lo, hi = boot_ci(ratios)
    unit = "µs" if kind == "p50" else "calls/s"
    print(f"| {label} | {len(ratios)} | {statistics.median(ins):,.0f} {unit} | {statistics.median(rpcs):,.0f} {unit} | {statistics.median(ratios):.2f} | {lo:.2f}-{hi:.2f} |")
spawn = {k: [blocks[b][r][f"{k}_spawn_to_first_reply"]["ms"] for b in complete if f"{k}_spawn_to_first_reply" in blocks[b][r]] for k, r in (("node_stdio", "stdio_node"), ("py_stdio", "stdio_py"))}
for k, v in spawn.items():
    if v:
        print(f"\n{k} sidecar spawn to first reply: median {statistics.median(v):.0f} ms over {len(v)} blocks")
