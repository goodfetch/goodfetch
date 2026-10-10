"""Summarises results/raw/<scenario>.jsonl as markdown. Paired metrics: per-block ratio,
median across blocks, bootstrap 95% CI (10,000 resamples), as in the sdk-binding bench.
Usage: analyze.py <s1|s2a|s2b|s2c|s3|s4|s5|feel>"""
import collections
import json
import os
import random
import statistics
import sys

R = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RAW = f"{R}/results/raw"


def load(scen):
    p = f"{RAW}/{scen}.jsonl"
    return [json.loads(x) for x in open(p)] if os.path.exists(p) else []


def boot_ci(xs, n=10000, seed=7):
    rnd = random.Random(seed)
    meds = sorted(statistics.median(rnd.choices(xs, k=len(xs))) for _ in range(n))
    return meds[int(0.025 * n)], meds[int(0.975 * n)]


def med(xs):
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else None


def f(x, d=2):
    if x is None:
        return "n/a"
    if isinstance(x, (int, float)) and not isinstance(x, bool) and abs(x) >= 1000:
        return f"{x:,.0f}"
    return f"{x:.{d}f}" if isinstance(x, float) else str(x)


def paired(rows_by_block, a_key, b_key, metric):
    """rows_by_block: {block: {key: row}} -> list of a/b ratios over blocks having both."""
    out = []
    for blk, m in sorted(rows_by_block.items()):
        a, b = m.get(a_key), m.get(b_key)
        if a and b and a.get(metric) and b.get(metric):
            out.append(a[metric] / b[metric])
    return out


def ratio_cell(rs):
    if len(rs) < 3:
        return f"{len(rs)} | n/a | n/a"
    lo, hi = boot_ci(rs)
    return f"{len(rs)} | **{statistics.median(rs):.2f}** | {lo:.2f}–{hi:.2f}"


def by_block(rows, keyf):
    d = collections.defaultdict(dict)
    for r in rows:
        if not r.get("failed"):
            d[r["block"]][keyf(r)] = r
    return d


def s1():
    rows = load("s1")
    bb = by_block(rows, lambda r: (r["runner"], r["variant"]))
    print("### S1 paired ratios (in-process ÷ RPC)\n")
    print("| Comparison | Blocks | Ratio (median) | 95% CI |")
    print("| --- | ---: | ---: | --- |")
    for v in ("fast-default", "fast-batched"):
        for ip, rpc in (("napi", "rpc-node"), ("pyo3", "rpc-py")):
            print(f"| {v}: throughput {ip}/{rpc} (>1: in-process faster) | {ratio_cell(paired(bb, (ip, v), (rpc, v), 'throughput_eps'))} |")
            print(f"| {v}: p50 latency {ip}/{rpc} (<1: in-process faster) | {ratio_cell(paired(bb, (ip, v), (rpc, v), 'p50_us'))} |")
    for ip in ("napi", "pyo3", "rpc-node", "rpc-py"):
        print(f"| {ip}: throughput batched/default | {ratio_cell(paired(bb, (ip, 'fast-batched'), (ip, 'fast-default'), 'throughput_eps'))} |")
    print("\n### S1 medians per runner and variant (100k events, 64 B payload; slow = 20 µs busy-wait per event)\n")
    print("| Runner | Variant | Throughput (ev/s) | p50 lat (ms) | p99 lat (ms) | Engine done emitting (ms) | Consumer done (ms) | Dropped | SDK peak RSS (MB) | engined peak RSS (MB) | Runs |")
    print("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
    g = collections.defaultdict(list)
    for r in rows:
        if not r.get("failed"):
            g[(r["runner"], r["variant"])].append(r)
    order = ["fast-default", "fast-batched", "fast-loop", "slow-default", "slow-loop", "slow-bounded", "slow-batched", "slow-drop"]
    for (rn, v), rs in sorted(g.items(), key=lambda kv: (["napi", "rpc-node", "pyo3", "rpc-py"].index(kv[0][0]), order.index(kv[0][1]))):
        rpc = rn.startswith("rpc")
        print(f"| {rn} | {v} | {f(med([x['throughput_eps'] for x in rs]))} | {f(med([x['p50_us'] / 1000 for x in rs]))} | {f(med([x['p99_us'] / 1000 for x in rs]))} | "
              f"{f(med([x['engine_emit_ms'] for x in rs]))} | {f(med([x['consume_ms'] for x in rs]))} | {f(med([x['dropped'] for x in rs]))} | "
              f"{f(med([x['sdk_peak_rss'] / 2**20 for x in rs]), 0)} | {f(med([x['engine_peak_rss'] / 2**20 for x in rs]), 0) if rpc else '(same process)'} | {len(rs)} |")
    fails = [r for r in rows if r.get("failed") or r.get("timed_out")]
    if fails:
        print(f"\nFailed/timed-out runs: {[(r['runner'], r.get('variant'), r['block']) for r in fails]}")


def s2a():
    rows = load("s2a")
    bb = by_block(rows, lambda r: r["runner"])
    print("### S2a cancel → run resolved (engine stopped), median of 5 cancels per run\n")
    print("| Runner | Median cancel latency (ms) | Min–max of per-run medians | Runs |")
    print("| --- | ---: | --- | ---: |")
    for rn in ("napi", "rpc-node", "pyo3", "rpc-py"):
        xs = [r["cancel_ms_p50"] for r in rows if r["runner"] == rn and "cancel_ms_p50" in r]
        if xs:
            print(f"| {rn} | {f(med(xs))} | {f(min(xs))}–{f(max(xs))} | {len(xs)} |")
    print("\n| Comparison | Blocks | Ratio in-process/RPC (median) | 95% CI |")
    print("| --- | ---: | ---: | --- |")
    for ip, rpc in (("napi", "rpc-node"), ("pyo3", "rpc-py")):
        print(f"| {ip}/{rpc} cancel latency | {ratio_cell(paired(bb, ip, rpc, 'cancel_ms_p50'))} |")


def s2b():
    rows = load("s2b")
    print("### S2b handlers that never resolve (20 pages, 4 hang, timeout 500 ms, concurrency 4)\n")
    print("| Runner | Done | Timed out | Run elapsed (ms, median) | Hung handlers cancelled in user code | Pending tasks after run | Process exit after main (ms) | Runs |")
    print("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
    for rn in ("napi", "rpc-node", "pyo3", "rpc-py"):
        rs = [r for r in rows if r["runner"] == rn and not r.get("failed")]
        if rs:
            print(f"| {rn} | {f(med([r['done'] for r in rs]))} | {f(med([r['timeout'] for r in rs]))} | {f(med([r['elapsed_ms'] for r in rs]), 0)} | "
                  f"{f(med([r['handler_cancellations_seen'] for r in rs]))} of {f(med([r['hung_handlers'] for r in rs]))} | {f(med([r.get('pending_tasks_after') for r in rs])) if rn in ('pyo3', 'rpc-py') else 'n/a'} | "
                  f"{f(med([r.get('exit_after_main_ms') for r in rs]), 0)} | {len(rs)} |")


def s2c():
    rows = load("s2c")
    print("### S2c one handler busy-loops 3 s (40 pages, concurrency 4, timeout 1 s)\n")
    print("| Runner | Done | Timed out | Run elapsed (ms) | Pages committed during the block | …of which timeouts | Max event lag (ms) | Runs |")
    print("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
    for rn in ("napi", "rpc-node", "pyo3", "rpc-py"):
        rs = [r for r in rows if r["runner"] == rn and not r.get("failed")]
        if rs:
            print(f"| {rn} | {f(med([r['done'] for r in rs]))} | {f(med([r['timeout'] for r in rs]))} | {f(med([r['elapsed_ms'] for r in rs]), 0)} | "
                  f"{f(med([r['committed_during_block'] for r in rs]))} | {f(med([r['timeouts_during_block'] for r in rs]))} | {f(med([r['max_event_lag_ms'] for r in rs]), 0)} | {len(rs)} |")


def s3():
    rows = load("s3")
    print("### S3 crash isolation (handler kills its process at page 100)\n")
    print("| Runner | Topology | Crash | SDK exit | integrity_check | Journal rows after | Acked pages | Acked but not durable | Notes |")
    print("| --- | --- | --- | ---: | --- | --- | ---: | ---: | --- |")
    for r in rows:
        if r["topology"] == "daemon":
            note = f"B: {r['rows_B']}; probe after crash: {'ok' if r['probe_after_crash'] else 'failed'}; " + "; ".join(x for x in r["daemon_log"] if "closed" in x)
            print(f"| {r['runner']} | daemon | {r['kind']} | {r['sdk_rc']} | {r['integrity']} | A: {r['rows_A']} | {r['acked_A']} | {r['acked_not_durable']} | {note} |")
        else:
            note = "; ".join(r["engine_log"]) or ("leases stay 'leased' (nobody left to release them)" if r["topology"] == "in-process" else "")
            if r.get("engined_exit_after_sdk_ms") is not None:
                note += f"; sidecar exited {r['engined_exit_after_sdk_ms']} ms after the SDK died"
            print(f"| {r['runner']} | {r['topology']} | {r['kind']} | {r['sdk_rc']} | {r['integrity']} | {r['rows']} | {r['acked']} | {r['acked_not_durable']} | {note} |")


def s4():
    rows = load("s4")
    bb = by_block(rows, lambda r: (r["runner"], r["variant"], r["size"]))
    sizes = sorted({r["size"] for r in rows})
    print("### S4 medians across blocks: time from engine handing the page over to handler entry (delivered) and to the full body in hand (body)\n")
    print("| Size | Runner | Variant | Delivered p50 (µs) | Body in hand p50 (µs) | Handler round trip p50 (µs) | Blocks |")
    print("| ---: | --- | --- | ---: | ---: | ---: | ---: |")
    g = collections.defaultdict(list)
    for r in rows:
        if not r.get("failed"):
            g[(r["size"], r["runner"], r["variant"])].append(r)
    for k in sorted(g, key=lambda k: (k[0], ["napi", "rpc-node", "pyo3", "rpc-py"].index(k[1]), k[2])):
        rs = g[k]
        print(f"| {k[0]:,} | {k[1]} | {k[2]} | {f(med([r['deliver_p50_us'] for r in rs]), 0)} | {f(med([r['body_p50_us'] for r in rs]), 0)} | {f(med([r['rt_p50_us'] for r in rs]), 0)} | {len(rs)} |")
    print("\n### S4 paired ratios on 'body in hand' vs RPC by reference (<1: faster than RPC by reference)\n")
    print("| Size | Comparison | Blocks | Ratio (median) | 95% CI |")
    print("| ---: | --- | ---: | ---: | --- |")
    for size in sizes:
        for a, b in ((("rpc-node", "inline"), ("rpc-node", "ref")), (("napi", "zero-copy"), ("rpc-node", "ref")), (("napi", "ref"), ("rpc-node", "ref")),
                     (("rpc-py", "inline"), ("rpc-py", "ref")), (("pyo3", "bytes"), ("rpc-py", "ref")), (("pyo3", "memoryview"), ("rpc-py", "ref")), (("pyo3", "ref"), ("rpc-py", "ref"))):
            print(f"| {size:,} | {a[0]} {a[1]} ÷ {b[0]} {b[1]} | {ratio_cell(paired(bb, (*a, size), (*b, size), 'body_p50_us'))} |")


def s5():
    rows = load("s5")
    g = collections.defaultdict(list)
    for r in rows:
        g[(r["lang"], r["topology"], r["n_procs"], r["inspector"])].append(r)
    print("### S5 medians across blocks (each process journals its pages; 1 KB pages, concurrency 8, synchronous=FULL)\n")
    print("| SDK | Topology | Procs | Inspector | Rows/s total | Commit p50 (ms) | Commit p99, worst process (ms) | SQLITE_BUSY after 5 s | Lock wait total (ms) | Commits | WAL at end (MB) | Inspector query p99 (ms) | Failures | Blocks |")
    print("| --- | --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
    for k in sorted(g):
        rs = g[k]
        ins = [r["inspector_stats"]["p99_ms"] for r in rs if r.get("inspector_stats")]
        bad = sum(1 for r in rs if r["rows"] != r["rows_expected"] or r["integrity"] != "ok" or r["proc_failures"])
        print(f"| {k[0]} | {k[1]} | {k[2]} | {'on' if k[3] else 'off'} | {f(med([r['throughput_rps'] for r in rs]), 0)} | {f(med([r['commit_p50_us_median'] / 1000 for r in rs]))} | "
              f"{f(med([r['commit_p99_us_max'] / 1000 for r in rs]))} | {f(med([r['busy_retries'] for r in rs]))} | {f(med([r['lock_wait_ms'] for r in rs]), 0)} | {f(med([r['commits'] for r in rs]))} | "
              f"{f(med([r['wal_bytes'] / 2**20 for r in rs]))} | {f(med(ins)) if ins else '-'} | {bad} | {len(rs)} |")
    bb = by_block(rows, lambda r: (r["lang"], r["topology"], r["n_procs"], r["inspector"]))
    print("\n### S5 paired ratios\n")
    print("| Comparison | Blocks | Ratio (median) | 95% CI |")
    print("| --- | ---: | ---: | --- |")
    for lang in ("node", "python"):
        for n in (1, 2, 4, 8):
            print(f"| {lang}, {n} procs: throughput daemon/embedded (>1: daemon faster) | {ratio_cell(paired(bb, (lang, 'daemon', n, False), (lang, 'embedded', n, False), 'throughput_rps'))} |")
            print(f"| {lang}, {n} procs: commit p99 daemon/embedded (<1: daemon lower) | {ratio_cell(paired(bb, (lang, 'daemon', n, False), (lang, 'embedded', n, False), 'commit_p99_us_max'))} |")
    for lang in ("node", "python"):
        for topo in ("embedded", "daemon"):
            for n in (1, 8):
                print(f"| {lang} {topo}, {n} procs: throughput with/without inspector | {ratio_cell(paired(bb, (lang, topo, n, True), (lang, topo, n, False), 'throughput_rps'))} |")


def feel():
    rows = load("feel")
    print("### Handler exception as seen by the engine\n")
    for r in rows:
        if r["scen"] in ("throw", "throwsync"):
            e = (r["rows"] or [{}])[0].get("error_seen_by_engine")
            print(f"- **{r['runner']} / {r['scen']}**: `{(e or 'NONE').splitlines()[0][:160]}` ({len((e or '').splitlines())} lines, stack frames: {'yes' if e and ('at ' in e or 'File ' in e) else 'no'})")
    for r in rows:
        if r["scen"] == "skew":
            print(f"- **{r['runner']} / version skew**: {(r['rows'] or [{}])[0]}")


if __name__ == "__main__":
    globals()[sys.argv[1]]()
