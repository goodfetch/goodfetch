"""Scenario runner for the Python SDKs. Usage: python scen.py <pyo3|rpc-py> <scenario> '<json args>'
Env: DATA (data dir), ENGINED (engined binary), SOCK (daemon socket; sidecar when unset)."""
import asyncio
import json
import os
import resource
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "sdk-py"))
from automan_sdk import now_us, read_body  # noqa: E402
from automan_sdk.rpc import RpcEngine  # noqa: E402

runner, scen = sys.argv[1], sys.argv[2]
A = json.loads(sys.argv[3]) if len(sys.argv) > 3 else {}
DATA = os.environ["DATA"]


def out(**o):
    print(json.dumps({"runner": runner, "scen": scen, **o}), flush=True)


def pct(xs, p):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(len(xs) * p))] if xs else 0


def busy(us):
    end = time.perf_counter_ns() + us * 1000
    while time.perf_counter_ns() < end:
        pass


def max_rss():
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss  # bytes on macOS


def base():
    return {"db_path": f"{DATA}/{A.get('db', 'engine.db')}", "blob_dir": f"{DATA}/blobs"}


async def open_engine(**kw):
    if runner == "pyo3":
        from automan_sdk.native import NativeEngine

        return NativeEngine(**kw)
    if os.environ.get("SOCK"):
        return await RpcEngine.connect(os.environ["SOCK"])
    return await RpcEngine.sidecar(os.environ["ENGINED"])


async def s1(eng):
    n, slow = A.get("n", 100_000), A.get("slow_us", 0)
    lats, last = [], [0.0]

    def on_event(evs):
        for e in evs:
            if e["kind"] != "tick":
                continue
            t = now_us()
            lats.append(t - e["t_us"])
            last[0] = t
            if slow:
                busy(slow)

    t0 = now_us()
    s = await eng.run({**base(), "stream_events": n, "event_bytes": 64, "event_mode": A.get("mode", "default"), "event_cap": 1024, "event_batch": A.get("batch", 1)}, lambda p: None, on_event)
    t_res = now_us()
    expected = s["events_emitted"] - s["events_dropped"]
    while len(lats) < expected and now_us() - t0 < 120e6:
        await asyncio.sleep(0.001)
    k, f = len(lats), s["t_first_emit_us"]
    out(variant=A.get("variant"), received=k, emitted=s["events_emitted"], dropped=s["events_dropped"],
        throughput_eps=k / ((last[0] - f) / 1e6) if k else 0, p50_us=pct(lats, 0.5), p99_us=pct(lats, 0.99),
        engine_emit_ms=(s["t_last_emit_us"] - f) / 1000, consume_ms=(last[0] - f) / 1000, run_resolved_ms=(t_res - f) / 1000,
        sdk_peak_rss=max_rss(), engine_peak_rss=s["engine_peak_rss"])


async def s2a(eng):
    ms, pages = [], []

    async def handler(p):
        await asyncio.sleep(0.001)

    for r in range(A.get("reps", 5)):
        h = eng.start({**base(), "run_id": f"s2a-{runner}-{os.getpid()}-{r}", "n_pages": 1_000_000, "page_bytes": 1024, "concurrency": 8, "item_events": False, "progress_every": 0}, handler)
        await asyncio.sleep(0.3)
        t = now_us()
        h.cancel()
        s = await h
        ms.append((now_us() - t) / 1000)
        pages.append(s["pages_done"])
        assert s["cancelled"]
    out(cancel_ms_p50=pct(ms, 0.5), cancel_ms=ms, pages_before_cancel=pages)


async def s2b(eng):
    st = {"hung": 0, "cancelled": 0}

    async def handler(p):
        if p["idx"] % 5 == 0:
            st["hung"] += 1
            try:
                await asyncio.Event().wait()
            except asyncio.CancelledError:
                st["cancelled"] += 1
                raise
        return {"ok": 1}

    t0 = now_us()
    s = await eng.run({**base(), "n_pages": 20, "page_bytes": 1024, "concurrency": 4, "handler_timeout_ms": 500, "item_events": False, "progress_every": 0}, handler)
    elapsed = (now_us() - t0) / 1000
    await asyncio.sleep(0.2)
    pending = len([t for t in asyncio.all_tasks() if t is not asyncio.current_task() and t is not getattr(eng, "_task", None)])
    out(done=s["pages_done"], timeout=s["pages_timeout"], elapsed_ms=elapsed, hung_handlers=st["hung"], handler_cancellations_seen=st["cancelled"], pending_tasks_after=pending, main_done_us=now_us())


async def s2c(eng):
    st = {"lag": 0.0, "n": 0, "bs": 0.0, "be": 0.0}

    async def handler(p):
        if p["idx"] == 3:
            st["bs"] = now_us()
            busy(3_000_000)  # holds the GIL and the event loop
            st["be"] = now_us()
        else:
            await asyncio.sleep(0.01)
        return {}

    def on_event(evs):
        for e in evs:
            st["lag"] = max(st["lag"], now_us() - e["t_us"])
            st["n"] += 1

    s = await eng.run({**base(), "n_pages": 40, "page_bytes": 1024, "concurrency": 4, "handler_timeout_ms": 1000, "item_events": True, "progress_every": 0, "timeline": True}, handler, on_event)
    w0, w1 = (st["bs"] - s["t_start_us"]) / 1000, (st["be"] - s["t_start_us"]) / 1000
    tl = s["timeline"]
    inb = [x for x in tl if w0 <= x[2] <= w1]
    out(done=s["pages_done"], timeout=s["pages_timeout"], timed_out_idx=[x[0] for x in tl if x[1] == "timeout"],
        elapsed_ms=(s["t_end_us"] - s["t_start_us"]) / 1000, block_ms=(st["be"] - st["bs"]) / 1000, committed_during_block=len(inb),
        timeouts_during_block=len([x for x in inb if x[1] == "timeout"]), max_event_lag_ms=st["lag"] / 1000, events=st["n"])


async def s3(eng):
    if getattr(eng, "info", None) and A.get("pidfile"):
        with open(A["pidfile"], "a") as f:
            f.write(f"{eng.info['pid']}\n")
    fd = os.open(A["ackfile"], os.O_WRONLY | os.O_CREAT | os.O_APPEND)

    async def handler(p):
        if p["idx"] == A.get("crash_at"):
            if A["kind"] == "exit":
                os._exit(1)
            if A["kind"] == "abort":
                os.abort()
        await asyncio.sleep(A.get("sleep_ms", 2) / 1000)
        return {}

    def on_event(evs):
        for e in evs:
            if e["kind"] == "item_done" and e["data"] == "done":
                os.write(fd, f"{e['idx']}\n".encode())

    s = await eng.run({**base(), "run_id": A["run_id"], "n_pages": A.get("n", 200), "page_bytes": 1024, "concurrency": 4, "leases": True, "item_events": True, "progress_every": 0}, handler, on_event)
    out(completed=True, done=s["pages_done"])


async def s4(_eng):
    variants = ["bytes", "memoryview", "ref"] if runner == "pyo3" else ["inline", "ref"]
    iters = {100_000: 60, 1_000_000: 20, 10_000_000: 6}

    async def handler(p):
        te = now_us()
        b = read_body(p)
        tb = now_us()
        return {"t_enter_us": te, "t_body_us": tb, "n": len(b)}

    for size in A.get("sizes", [100_000, 1_000_000, 10_000_000]):
        for v in variants:
            eng = await open_engine(body=v) if runner == "pyo3" else _eng
            s = await eng.run({**base(), "run_id": f"s4-{runner}-{os.getpid()}-{v}-{size}", "n_pages": iters.get(size, 10), "page_bytes": size, "concurrency": 1, "inline": v != "ref", "item_events": False, "progress_every": 0}, handler)
            out(variant=v, size=size, n=s["pages_done"], deliver_p50_us=s["deliver_p50_us"], body_p50_us=s["body_p50_us"], rt_p50_us=s["rt_p50_us"])


async def s5(eng):
    async def handler(p):
        return {}

    s = await eng.run({**base(), "run_id": A["run_id"], "n_pages": A.get("n", 500), "page_bytes": 1024, "concurrency": 8, "item_events": False, "progress_every": 0}, handler)
    out(rows=s["pages_done"], t_start_us=s["t_start_us"], t_end_us=s["t_end_us"], commit_p50_us=s["commit_p50_us"], commit_p99_us=s["commit_p99_us"],
        writer_busy=s["writer_busy"], writer_commits=s["writer_commits"], writer_lock_wait_ms=s["writer_lock_wait_ms"])


async def throw(eng):
    def parse_listing(html):
        raise ValueError("boom: selector .price not found")

    async def handler(p):
        return parse_listing(read_body(p))

    s = await eng.run({**base(), "n_pages": 1, "page_bytes": 1024, "item_events": False, "progress_every": 0}, handler)
    out(error_seen_by_engine=s["errors"][0] if s["errors"] else None)


async def throwsync(eng):
    def handler(p):  # not async: raises when the engine calls it
        raise ValueError("sync boom")

    s = await eng.run({**base(), "n_pages": 1, "page_bytes": 1024, "item_events": False, "progress_every": 0}, handler)
    out(error_seen_by_engine=s["errors"][0] if s["errors"] else None)


async def probe(eng):
    out(info=getattr(eng, "info", None))


async def main():
    if scen == "skew":
        try:
            await RpcEngine.sidecar(os.environ["ENGINED"], "2.0")
            out(error=None)
        except Exception as e:  # noqa: BLE001
            out(error=str(e), code=getattr(e, "code", None))
        os._exit(0)
    kw = {"event_delivery": A["delivery"]} if runner == "pyo3" and "delivery" in A else {}
    eng = await open_engine(**kw)
    await globals()[scen](eng)
    await eng.close()


asyncio.run(main())
