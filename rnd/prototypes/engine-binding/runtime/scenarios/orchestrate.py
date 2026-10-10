"""Runs the scenarios as paired blocks (runner order shuffled per block) and appends raw
results to results/raw/<scenario>.jsonl. Usage: orchestrate.py <s1|s2a|s2b|s2c|s3|s4|s5|feel> [--blocks N]"""
import argparse
import json
import os
import random
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

R = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ENGINED = f"{R}/target/release/engined"
PY = f"{R}/.venv/bin/python"
RAW = f"{R}/results/raw"
RUNNERS = ["napi", "rpc-node", "pyo3", "rpc-py"]


def cmd(runner, scen, args):
    if runner in ("napi", "rpc-node"):
        return ["node", f"{R}/scenarios/scen.ts", runner, scen, json.dumps(args)]
    return [PY, f"{R}/scenarios/scen.py", runner, scen, json.dumps(args)]


def env(data, sock=None):
    e = dict(os.environ, DATA=data, ENGINED=ENGINED)
    e.pop("SOCK", None)
    if sock:
        e["SOCK"] = sock
    return e


def fresh(name):
    d = f"{R}/data/{name}"
    shutil.rmtree(d, ignore_errors=True)  # scratch created by this harness
    os.makedirs(d)
    return d


def parse(out):
    return [json.loads(line) for line in out.splitlines() if line.startswith("{")]


def run1(runner, scen, args, data, sock=None, timeout=300):
    t = time.time()
    p = subprocess.Popen(cmd(runner, scen, args), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env(data, sock))
    to = False
    try:
        o, e = p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        p.kill()
        o, e = p.communicate()
        to = True
    return parse(o), {"rc": p.returncode, "wall_s": round(time.time() - t, 3), "timed_out": to, "exit_us": time.time_ns() / 1000, "stderr": e[-1500:]}


def record(scen, row):
    os.makedirs(RAW, exist_ok=True)
    with open(f"{RAW}/{scen}.jsonl", "a") as f:
        f.write(json.dumps(row) + "\n")
    brief = {k: v for k, v in row.items() if k not in ("stderr", "cancel_ms", "pages_before_cancel", "timed_out_idx", "done_idx")}
    print("  " + json.dumps(brief)[:400], flush=True)


def meta(block):
    return {"block": block, "load1": round(os.getloadavg()[0], 1), "ts": time.strftime("%Y-%m-%dT%H:%M:%S")}


def shuffled(xs, block, salt=0):
    xs = list(xs)
    random.Random(1000 * block + salt).shuffle(xs)
    return xs


# ------------------------------------------------------------------ S1
S1_VARIANTS = {
    "fast-default": {"mode": "default"},
    "fast-batched": {"mode": "bounded", "batch": 256},
    "slow-default": {"mode": "default", "slow_us": 20},
    "slow-bounded": {"mode": "bounded", "slow_us": 20},
    "slow-drop": {"mode": "drop", "slow_us": 20},
    "slow-batched": {"mode": "bounded", "batch": 256, "slow_us": 20},
}


def s1(blocks):
    for b in range(1, blocks + 1):
        jobs = [(r, v) for r in RUNNERS for v in S1_VARIANTS]
        jobs += [("pyo3", "fast-loop"), ("pyo3", "slow-loop")]
        for r, v in shuffled(jobs, b):
            a = dict(S1_VARIANTS.get(v) or {"mode": "default", "delivery": "loop", **({"slow_us": 20} if v.startswith("slow") else {})})
            a.update(variant=v, n=100_000)
            rows, info = run1(r, "s1", a, fresh("s1"), timeout=300)
            for row in rows or [{"runner": r, "variant": v, "failed": True}]:
                record("s1", {**meta(b), **row, "variant": v, "rc": info["rc"], "timed_out": info["timed_out"], "wall_s": info["wall_s"]})


# ------------------------------------------------------------------ S2
def s2(scen, blocks):
    for b in range(1, blocks + 1):
        for r in shuffled(RUNNERS, b, {"s2a": 2, "s2b": 3, "s2c": 6}[scen]):
            rows, info = run1(r, scen, {"reps": 5}, fresh(scen), timeout=120)
            for row in rows or [{"runner": r, "failed": True}]:
                extra = {}
                if "main_done_us" in row:
                    extra["exit_after_main_ms"] = round((info["exit_us"] - row["main_done_us"]) / 1000, 1)
                record(scen, {**meta(b), **row, **extra, "rc": info["rc"], "timed_out": info["timed_out"], "wall_s": info["wall_s"], "stderr": info["stderr"][-300:]})


# ------------------------------------------------------------------ S3
def check(db):
    return json.loads(subprocess.run([ENGINED, "check", db], capture_output=True, text=True).stdout)


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def counts_for(chk, run_id):
    return {s: n for (r, s, n) in chk["counts"] if r == run_id}


def acked(path):
    return {int(x) for x in open(path).read().split()} if os.path.exists(path) else set()


def s3(_blocks):
    for r in RUNNERS:
        for kind in ("exit", "abort"):
            d = fresh(f"s3-{r}-{kind}")
            ack, pidf = f"{d}/ack.txt", f"{d}/engined.pid"
            open(ack, "w").close()
            rows, info = run1(r, "s3", {"kind": kind, "crash_at": 100, "n": 200, "ackfile": ack, "pidfile": pidf, "run_id": "crash"}, d, timeout=120)
            t_dead = time.time()
            engined_exit_ms = None
            if os.path.exists(pidf):
                pid = int(open(pidf).read().split()[0])
                while alive(pid) and time.time() - t_dead < 15:
                    time.sleep(0.01)
                engined_exit_ms = None if alive(pid) else round((time.time() - t_dead) * 1000)
            chk = check(f"{d}/engine.db")
            done = {i for (run, i) in chk["done"] if run == "crash"}
            ak = acked(ack)
            record("s3", {**meta(0), "runner": r, "topology": "in-process" if r in ("napi", "pyo3") else "sidecar", "kind": kind,
                          "sdk_rc": info["rc"], "integrity": chk["integrity"], "rows": counts_for(chk, "crash"), "acked": len(ak),
                          "acked_not_durable": len(ak - done), "engined_exit_after_sdk_ms": engined_exit_ms,
                          "engine_log": [l for l in info["stderr"].splitlines() if "[engined]" in l][-2:]})
    for a_runner, a_kind, b_runner in (("rpc-node", "exit", "rpc-py"), ("rpc-py", "abort", "rpc-node")):
        d = fresh(f"s3-daemon-{a_runner}")
        sock = os.path.join(tempfile.mkdtemp(prefix="am"), "s")
        dlog = open(f"{d}/daemon.log", "w")
        daemon = subprocess.Popen([ENGINED, "serve", sock], stderr=dlog, stdout=subprocess.DEVNULL)
        while not os.path.exists(sock):
            time.sleep(0.01)
        ack_a, ack_b = f"{d}/ackA.txt", f"{d}/ackB.txt"
        pa = subprocess.Popen(cmd(a_runner, "s3", {"kind": a_kind, "crash_at": 100, "n": 2000, "ackfile": ack_a, "run_id": "A", "sleep_ms": 2}), env=env(d, sock), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        pb = subprocess.Popen(cmd(b_runner, "s3", {"kind": "none", "n": 300, "ackfile": ack_b, "run_id": "B", "sleep_ms": 5}), env=env(d, sock), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        a_out, _ = pa.communicate(timeout=180)
        b_out, _ = pb.communicate(timeout=180)
        probe, _ = run1("rpc-node", "probe", {}, d, sock=sock, timeout=60)
        time.sleep(0.5)
        daemon.terminate()
        daemon.wait()
        dlog.close()
        chk = check(f"{d}/engine.db")
        da = {i for (run, i) in chk["done"] if run == "A"}
        record("s3", {**meta(0), "runner": f"daemon: {a_runner} crashes ({a_kind}), {b_runner} keeps running", "topology": "daemon", "kind": a_kind,
                      "sdk_rc": pa.returncode, "integrity": chk["integrity"], "rows_A": counts_for(chk, "A"), "rows_B": counts_for(chk, "B"),
                      "acked_A": len(acked(ack_a)), "acked_not_durable": len(acked(ack_a) - da), "B_completed": parse(b_out),
                      "probe_after_crash": probe, "daemon_log": open(f"{d}/daemon.log").read().splitlines()})


# ------------------------------------------------------------------ S4
def s4(blocks):
    for b in range(1, blocks + 1):
        for r in shuffled(RUNNERS, b, 4):
            rows, info = run1(r, "s4", {}, fresh("s4"), timeout=600)
            for row in rows or [{"runner": r, "failed": True}]:
                record("s4", {**meta(b), **row, "rc": info["rc"]})


# ------------------------------------------------------------------ S5
S5_TOPO = {("node", "embedded"): "napi", ("node", "daemon"): "rpc-node", ("python", "embedded"): "pyo3", ("python", "daemon"): "rpc-py"}


def s5_once(lang, topo, n, insp, pages):
    d = fresh("s5")
    db = f"{d}/shared.db"
    sock, daemon, ip = None, None, None
    if topo == "daemon":
        sock = os.path.join(tempfile.mkdtemp(prefix="am"), "s")
        daemon = subprocess.Popen([ENGINED, "serve", sock], stderr=subprocess.DEVNULL)
        while not os.path.exists(sock):
            time.sleep(0.01)
    if insp:
        ip = subprocess.Popen([ENGINED, "inspect", db, "50"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    runner = S5_TOPO[(lang, topo)]
    t0 = time.time()
    procs = [subprocess.Popen(cmd(runner, "s5", {"run_id": f"p{i}", "n": pages, "db": "shared.db"}), env=env(d, sock), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) for i in range(n)]
    rows, errs = [], 0
    for p in procs:
        try:
            o, _ = p.communicate(timeout=300)
        except subprocess.TimeoutExpired:
            p.kill()
            o, _ = p.communicate()
        rows += parse(o)
        errs += p.returncode != 0
    wall = time.time() - t0
    insp_out = None
    if ip:
        o, _ = ip.communicate(input="", timeout=60)
        insp_out = (parse(o) or [None])[0]
    if daemon:
        daemon.terminate()
        daemon.wait()
    wal = os.path.getsize(db + "-wal") if os.path.exists(db + "-wal") else 0
    chk = check(db)
    total = sum(r["rows"] for r in rows)
    span = (max(r["t_end_us"] for r in rows) - min(r["t_start_us"] for r in rows)) / 1e6 if rows else 0
    return {"lang": lang, "topology": topo, "n_procs": n, "inspector": insp, "rows": total, "rows_expected": n * pages, "proc_failures": errs,
            "throughput_rps": total / span if span else 0, "span_s": span, "wall_s": wall,
            "commit_p50_us_median": statistics.median(r["commit_p50_us"] for r in rows) if rows else None,
            "commit_p99_us_max": max(r["commit_p99_us"] for r in rows) if rows else None,
            "busy_retries": (sum if topo == "embedded" else max)(r["writer_busy"] for r in rows) if rows else None,
            "lock_wait_ms": (sum if topo == "embedded" else max)(r["writer_lock_wait_ms"] for r in rows) if rows else None,
            "commits": (sum if topo == "embedded" else max)(r["writer_commits"] for r in rows) if rows else None,
            "wal_bytes": wal, "integrity": chk["integrity"], "inspector_stats": insp_out}


def s5(blocks, pages=1000):
    for b in range(1, blocks + 1):
        cfgs = [(lang, topo, n, insp) for lang in ("node", "python") for topo in ("embedded", "daemon") for n in (1, 2, 4, 8) for insp in (False, True)]
        for lang, topo, n, insp in shuffled(cfgs, b, 5):
            record("s5", {**meta(b), **s5_once(lang, topo, n, insp, pages)})


# ------------------------------------------------------------------ feel
def feel(_blocks):
    for r in RUNNERS:
        for scen in ("throw", "throwsync"):
            rows, info = run1(r, scen, {}, fresh("feel"), timeout=60)
            record("feel", {"runner": r, "scen": scen, "rows": rows, "rc": info["rc"], "stderr": info["stderr"][-600:]})
    for r in ("rpc-node", "rpc-py"):
        rows, info = run1(r, "skew", {}, fresh("feel"), timeout=60)
        record("feel", {"runner": r, "scen": "skew", "rows": rows, "rc": info["rc"]})


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("scenario")
    ap.add_argument("--blocks", type=int, default=5)
    a = ap.parse_args()
    if os.path.exists(f"{RAW}/{a.scenario}.jsonl"):
        os.remove(f"{RAW}/{a.scenario}.jsonl")  # regenerated output of this harness
    print(f"## {a.scenario}: {a.blocks} block(s), start {time.strftime('%H:%M:%S')}, load {os.getloadavg()}", flush=True)
    {"s1": s1, "s2a": lambda n: s2("s2a", n), "s2b": lambda n: s2("s2b", n), "s2c": lambda n: s2("s2c", n), "s3": s3, "s4": s4, "s5": s5, "feel": feel}[a.scenario](a.blocks)
    print(f"## {a.scenario} done {time.strftime('%H:%M:%S')}", flush=True)
