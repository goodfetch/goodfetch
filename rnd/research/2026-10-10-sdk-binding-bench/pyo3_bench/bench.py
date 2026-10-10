"""In-process Python <-> Rust (PyO3 + pyo3-async-runtimes/Tokio) microbenchmark."""
import asyncio
import json
import platform
import sys
import time

import pyo3_bench as rb


def make_html(n):
    unit = '<div class="item" data-id="42"><a href="/p?q=1&r=2">Item élève</a></div>\n'
    s = ""
    while len(s.encode()) < n:
        s += unit
    return s[:n]


def time_sync(label, n, fn):
    for i in range(min(n, 10000)):
        fn(i)
    t0 = time.perf_counter_ns()
    for i in range(n):
        fn(i)
    print(json.dumps({"bench": label, "n": n, "mean_ns": (time.perf_counter_ns() - t0) / n}))


async def time_async(label, n, fn):
    for i in range(min(n, 2000)):
        await fn(i)
    samples = []
    for i in range(n):
        t0 = time.perf_counter_ns()
        await fn(i)
        samples.append(time.perf_counter_ns() - t0)
    samples.sort()
    print(json.dumps({"bench": label, "n": n, "mean_ns": sum(samples) / n, "p50_ns": samples[n // 2], "p99_ns": samples[int(n * 0.99)]}))


async def handler(html):
    return json.dumps({"n": len(html)})


def py_noop(i):
    return i + 1


async def main():
    print(json.dumps({"python": sys.version.split()[0], "machine": platform.machine()}))
    time_sync("py_noop_baseline", 2_000_000, py_noop)
    time_sync("pyo3_sync_noop", 2_000_000, rb.noop)
    for size in (64, 100_000, 1_000_000):
        s = make_html(size)
        b = s.encode()
        n = 2000 if size >= 1_000_000 else 20000 if size >= 100_000 else 1_000_000
        time_sync(f"pyo3_take_string_{size}", n, lambda i: rb.take_string(s))
        time_sync(f"pyo3_take_bytes_{size}", n, lambda i: rb.take_bytes(b))
    await time_async("py_await_coroutine_baseline", 200_000, lambda i: handler("x"))
    await time_async("pyo3_async_fn_tokio_roundtrip", 200_000, rb.noop_async)
    for size in (64, 100_000, 1_000_000):
        iters = 2000 if size >= 1_000_000 else 10000 if size >= 100_000 else 50000
        await rb.bench_handler(handler, size, 500)
        print(json.dumps(await rb.bench_handler(handler, size, iters)))


asyncio.run(main())
