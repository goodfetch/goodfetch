# Engine binding: native vs RPC, in practice

> **PROTOTYPE, throwaway.** Built for wayfinder ticket [#74](https://github.com/Jbmanllr/automan/issues/74). Nothing here is an adopted decision, a term or a design. The code has no tests and no error handling beyond what it needs to run. It is never merged.

**Question.** What do in-process native bindings (napi-rs for Node, PyO3 for Python) and an RPC engine process (a Rust `engined` speaking JSON-RPC 2.0, as a stdio sidecar or a Unix-socket daemon) feel like and cost for a Rust/Tokio scraping engine with TypeScript and Python SDKs? This covers event streaming, cancellation, crash isolation, pages by reference, several processes sharing one SQLite file, and the developer experience.

This prototype does not re-measure small handler round trips. The [paired SDK-binding bench](https://github.com/Jbmanllr/automan/tree/research/sdk-binding/rnd/research/2026-10-10-sdk-binding-bench) already did: in-process is about 2x faster for a 64 B call, and inline 1 MB pages cost 3–12x more over a pipe.

## What was built

| Part | What it does |
| --- | --- |
| `engine-core/` | Stub engine. `run(opts, host, cancel)` generates fake pages and stores each body in a content-addressed blob directory named by sha256. It calls the handler with the body inline or a `{path, size, sha256}` reference, under an optional engine-side timeout. It journals every page to one SQLite file (WAL, `synchronous=FULL`, `busy_timeout` 5000, one writer thread per file per process, group commit of up to 512 rows per transaction) and optionally writes a `leased` row first. It emits events and checks a `CancellationToken` between steps. Event delivery modes: `default` (each binding's natural mechanism), `bounded` (engine queue of 1024 that blocks when full, with a pump that waits for delivery and an optional batch of N events), and `drop`. |
| `napi-addon/` + `sdk-ts/native.ts` | napi-rs 3.14: `ThreadsafeFunction` for the handler (`call_async_catch` + awaiting the returned Promise) and for events (`NonBlocking` by default, `call_async` when bounded). Pages arrive as a zero-copy external `Buffer`; cancellation through an `AbortSignal` that calls `cancel(runId)`. |
| `pyo3-ext/` + `sdk-py/automan_sdk/native.py` | PyO3 0.29 with abi3-py311. The engine runs on a Tokio runtime owned by the extension. Handlers are scheduled on the caller's asyncio loop with `run_coroutine_threadsafe`, and a drop guard cancels them. Events are either called directly from the engine thread under the GIL (`direct`, the default) or passed to the loop (`call_soon_threadsafe`). The body arrives as `bytes` (one copy) or as a zero-copy `memoryview` over Rust memory through the buffer protocol. Cancellation uses a token object. |
| `engined/` + `sdk-ts/rpc.ts` + `sdk-py/automan_sdk/rpc.py` | Newline-delimited JSON-RPC 2.0. `engined stdio` is the sidecar mode and `engined serve <sock>` the daemon mode. `initialize` checks the protocol version. `run` is one long request. The engine calls handlers with `handle` requests sent to the client and streams events as `events` notifications. Cancellation uses `$/cancelRequest` in both directions; the engine sends one when a handler times out. When a client disconnects, its runs are cancelled and its leases re-queued. `engined inspect` is a read-only poller similar to a `status` CLI, and `engined check` runs `integrity_check` and counts rows. |
| `scenarios/` | `scen.ts` / `scen.py` (one runner and one scenario per process), `orchestrate.py` (paired blocks with runner order shuffled per block) and `analyze.py` (median of per-block ratios, bootstrap 95% CI with 10,000 resamples, the same method as `analyze_paired.py`). |

Run everything with `./run.sh`. Pass scenario names (`./run.sh --no-build s1 s4`) to run a subset and `--build-feel` for clean-build timings. Raw rows are in `results/raw/*.jsonl` and the printed summaries in `results/summary_*.md`. The four runners are **napi** (Node, in-process), **rpc-node** (Node SDK and engined), **pyo3** (Python, in-process) and **rpc-py** (Python SDK and engined).

## Method and caveats

- **Host:** Apple M3 Max (14 cores), macOS 27 (beta toolchain: Xcode SDK 27.0, ld-27037), Node 24.19.0, CPython 3.13.11, Rust 1.93.0. Release builds without LTO.
- **Load:** the host was **very heavily loaded by unrelated work**. Load average was 90–1,020 across the runs: S1 206–1,016, S2 113–191, S4 94–109 and S5 90–1,023. **Read ratios first.** Each block runs all runners back to back in a shuffled order, so drift partly cancels. Absolute numbers are indicative only, and tails (p99) are noisy. This host is lab class `local-dev`, not a reference host.
- **Blocks:** S1, S2a, S4 and S5 used 5 paired blocks each. S2b and S2c used 3 blocks; they are qualitative. S3 ran once per crash type.
- **macOS, not Linux.** `synchronous=FULL` on macOS calls `fsync` without `F_FULLFSYNC`, which does not flush the drive cache, so commits are cheaper here than on Linux with a real `fsync`. S5's commit costs are therefore optimistic. **Windows was not tested.** Named pipes instead of Unix sockets, and process-kill semantics, are untested.
- **Not covered:** installing on a clean machine, which the ticket mentions, was not measured. Prebuilt artifacts were not produced. Browser processes and real network I/O are stubbed.

## S1: event streaming (100k events of 64 B)

Default delivery per binding, as shipped:

- **napi:** `ThreadsafeFunction` call in `NonBlocking` mode into its **unbounded** queue (napi's default `max_queue_size` is 0). The engine never waits.
- **PyO3 `direct`:** the engine thread takes the GIL and runs the callback. The **engine waits** for each event, and the user callback runs on a Tokio thread, not on the asyncio loop.
- **PyO3 `loop`:** `call_soon_threadsafe` into asyncio's **unbounded** ready queue.
- **RPC:** an **unbounded** outgoing queue inside engined. A client whose consumer is synchronous stops reading, so the kernel socket buffer fills and the backlog piles up in engined.

The slow consumer busy-waits 20 µs per event. The table shows medians over 5 blocks.

| Runner | Variant | Engine done emitting | Consumer done | Dropped | Peak RSS where the backlog sits | p50 / p99 event latency |
| --- | --- | ---: | ---: | ---: | --- | --- |
| napi | slow, default | **46 ms** | 6.6 s | 0 | SDK 114 MB vs 90 MB bounded (**+24 MB**, in the TSFN queue) | 2.7 s / 6.5 s |
| napi | slow, bounded (batch 1) | 37.8 s | 38.0 s | 0 | 90 MB | 140 ms / 2.3 s |
| napi | slow, bounded + batch 256 | 6.2 s | 6.3 s | 0 | 96 MB | 57 ms / 170 ms |
| napi | slow, drop | 29 ms | 115 ms | **98,976** | 91 MB | 82 ms / 113 ms |
| rpc-node | slow, default | **486 ms** | 9.2 s | 0 | engined **32 MB** vs 4–5 MB bounded | 4.6 s / 8.5 s |
| rpc-node | slow, bounded + batch 256 | 6.9 s | 6.9 s | 0 | engined 5 MB | 84 ms / 439 ms |
| pyo3 | slow, default (`direct`) | 6.7 s | 6.7 s | 0 | 30 MB (engine blocked, no backlog) | 0.00 / 0.05 ms |
| pyo3 | slow, `loop` | 6.1 s | 11.4 s | 0 | SDK **59 MB** vs 30 MB (**+29 MB**, in asyncio's queue) | 1.8 s / 2.8 s |
| pyo3 | slow, bounded + batch 256 | 6.5 s | 6.5 s | 0 | 34 MB | 2.3 ms / 91 ms |
| rpc-py | slow, default | 359 ms | 5.7 s | 0 | engined 32 MB | 3.8 s / 5.4 s |
| rpc-py | slow, bounded + batch 256 | 4.3 s | 4.3 s | 0 | engined 5 MB | 52 ms / 309 ms |

Fast consumer, paired in-process ÷ RPC (5 blocks). A throughput ratio above 1 or a latency ratio below 1 means in-process is better.

| Comparison | Ratio (median) | 95% CI |
| --- | ---: | --- |
| Throughput, default, napi / rpc-node | 1.46 | 0.36–4.81 |
| Throughput, default, pyo3 / rpc-py | 1.18 | 0.12–2.39 |
| Throughput, batched 256, napi / rpc-node | 0.94 | 0.12–1.15 |
| Throughput, batched 256, pyo3 / rpc-py | 0.65 | 0.10–6.35 |
| Throughput batched / default, per runner (napi, rpc-node, pyo3, rpc-py) | 1.78, 2.50, 6.81, 2.51 | 0.36–5.51, 0.87–13.76, 0.04–49.45, 0.68–35.96 |

Fast-consumer medians:

- **Throughput:** 170k events/s for napi, 127k for rpc-node, 135k for pyo3 and 81k for rpc-py in default mode. Batched: 235k, 371k, 406k and 166k.
- **p50 latency:** in default mode, napi, rpc-node and rpc-py measure 0.29–0.63 s. That is backlog, because the engine emits 100k events faster than any consumer drains them. Batching brings it down to 2–4 ms. pyo3 `direct` measures µs because the engine waits for each event.

**Reading.**

- **Bounded memory is not anyone's default.** napi, PyO3 in `loop` mode and RPC all buffer without bound by default. The backlog sits in the SDK process for the two native bindings and in engined for RPC. PyO3 in `direct` mode is bounded only because it blocks an engine thread and runs user code on a foreign thread.
- **What bounded backpressure takes is the same in every binding:**
  - an engine-side bounded queue;
  - a delivery that completes only once the consumer has the event: `call_async` for napi, a direct GIL call for PyO3, and writing plus flushing to the socket for RPC;
  - and batching. Without batching, waiting per event costs 2.6–8.7k events/s; with batches of 256, the engine runs at the consumer's speed.
- **Native TSFN alternative:** napi's own option, `max_queue_size::<N>()` with `Blocking` mode, blocks the calling Tokio worker thread. It would need `spawn_blocking`, and was not used.
- **Async consumers over RPC need more:** an RPC client with an *async* consumer keeps reading, so it would buffer on the client side. That would need credit- or ack-based flow control in the protocol (not built).
- **Ratios:** the in-process vs RPC throughput ratios are not resolved at this load; every CI spans 1. Batching is the large lever on every runner.

## S2: cancellation

**(a) Cancel a running run (8 concurrent 1 ms handlers):** time from `cancel()` until the run promise resolves with `cancelled: true`. The engine stops at its next check and abandons in-flight handlers.

| Runner | Median (ms) | Paired ratio in-process / RPC | 95% CI |
| --- | ---: | ---: | --- |
| napi | 0.38 | 1.05 (napi / rpc-node) | 0.60–4.67 |
| rpc-node | 0.36 | | |
| pyo3 | 0.40 | 0.88 (pyo3 / rpc-py) | 0.50–1.04 |
| rpc-py | 0.52 | | |

There is no measurable difference: all four are sub-millisecond.

**(b) Handlers that never resolve** (20 pages, 4 handlers hang, 500 ms engine timeout). In every binding the engine timed out the 4 hung handlers and finished the other 16 in 0.56–0.65 s. Each process exited normally 19–32 ms after its main returned; a pending promise or task does not keep Node or Python alive. What differs is whether the hung user code is cancelled:

- **napi:** no, 0 of 4. A JS Promise cannot be cancelled from Rust; that would need a per-call `AbortController` plumbed through a second TSFN. The promises simply leak.
- **PyO3:** yes, 4 of 4, with 0 pending tasks left. But `pyo3-async-runtimes`' `into_future` does **not** cancel the Python task when the Rust future is dropped. It had to be replaced by `run_coroutine_threadsafe` plus `cf.cancel()` in a drop guard.
- **RPC:** yes, 4 of 4. The engine's `$/cancelRequest` becomes an `AbortSignal` in Node or `task.cancel()` in Python.

**(c) One handler busy-loops for 3 s** (40 pages, concurrency 4, 1 s timeout). The result is identical in all four bindings: the engine keeps running and committing. During the 3 s block it committed 8 pages, all **timeouts**, because handlers queued behind the blocked event loop could not run. In-process does not stall the engine either, since the engine has its own threads; neither topology rescues user code that blocks its own loop.

- **Event lag:** events reached the user about 2 s late on napi, rpc-node and rpc-py; they queue until the loop is free.
- **PyO3 `direct` exception:** events kept arriving with at most 12 ms lag. The engine thread takes the GIL at each switch interval and runs the user's callback *concurrently* with the blocked asyncio code. That looks good, but it means the callback runs off-loop, so touching asyncio objects from it is unsafe.

## S3: crash isolation (the handler kills its process at page 100 of 200; leases on)

| Runner | Crash | integrity_check | Journal after | Acked pages not durable | Leases |
| --- | --- | --- | --- | ---: | --- |
| napi (in-process) | `process.exit(1)` / `process.abort()` | ok / ok | 99 / 100 done, 4 leased | 0 / 0 | **4 stuck `leased`**: nobody is left to release them; it would need lease expiry on the next open |
| pyo3 (in-process) | `os._exit(1)` / `os.abort()` | ok / ok | 100 / 100 done, 4 leased | 0 / 0 | **4 stuck `leased`** |
| rpc-node (sidecar) | exit / abort | ok / ok | 100 / 98 done, 4 queued | 0 / 0 | engined saw EOF, **re-queued 4 in 0.2–0.4 ms**, and exited at once |
| rpc-py (sidecar) | exit / abort | ok / ok | 100 / 100 done, 4 queued | 0 / 0 | same |
| daemon: rpc-node client A crashes (exit) while rpc-py client B runs 300 pages | exit | ok | A: 97 done, 4 queued; **B: 300 / 300 done** | 0 | the daemon re-queued A's 4 leases in 0.3 ms, kept serving B, and accepted a new client afterwards |
| daemon: rpc-py client A crashes (abort) while rpc-node client B runs | abort | ok | A: 100 done, 4 queued; **B: 300 / 300 done** | 0 | same, 0.3 ms |

"Acked" means an `item_done` event that the SDK appended to a file before it died. The engine emits it only after the commit.

- **SQLite was consistent in every case.** WAL with `synchronous=FULL` survived `exit` and `abort` of a process that contained the writer thread. No acknowledged page was lost.
- **What the process boundary buys is lease recovery and isolation from other clients, not database safety.**
- **A bug the prototype surfaced:** the first S3 pass recorded 2–4 in-flight pages per crash as `error` instead of re-queuing them. On disconnect, the pending handler calls failed with "client gone" before the run saw its cancellation. The engine must tell *transport lost* apart from *handler failed*. Here that took cancellation-first ordering (`biased` select, plus discarding errors once cancelled).

## S4: pages by reference (100 KB, 1 MB, 10 MB; concurrency 1)

"Body in hand" is the time from the engine handing over the page until the handler has the full body: an inline argument, or a completed `readFileSync` / `open().read()` of the blob, served from the page cache. The ratio is against **RPC by reference** in the same language, which is the current direction. Below 1 means faster than RPC by reference.

| Size | RPC inline ÷ RPC ref (Node / Py) | In-process zero-copy ÷ RPC ref (napi Buffer / pyo3 memoryview) | pyo3 `bytes` ÷ RPC ref | In-process ref ÷ RPC ref (napi / pyo3) |
| ---: | --- | --- | --- | --- |
| 100 KB | **2.75** (1.80–3.31) / **2.94** (2.65–3.10) | 0.17 (0.08–0.22) / 0.34 (0.23–0.45) | 0.38 (0.26–0.51) | 0.66 (0.29–0.77) / 0.63 (0.43–0.87) |
| 1 MB | **9.10** (7.21–10.51) / **9.81** (7.82–11.67) | 0.12 (0.10–0.16) / 0.36 (0.20–0.52) | 0.35 (0.24–0.58) | 0.86 (0.55–1.10) / 0.87 (0.48–1.37) |
| 10 MB | **14.78** (13.85–18.85) / **17.24** (15.84–19.87) | 0.04 (0.04–0.05) / 0.11 (0.09–0.11) | 0.29 (0.22–0.34) | 1.03 (0.85–1.07) / 0.97 (0.92–1.04) |

Indicative medians at 10 MB, body in hand:

- **Node:** RPC inline 22 ms, RPC by reference 1.5 ms, in-process by reference 1.5 ms, napi zero-copy Buffer 0.07 ms.
- **Python:** RPC inline 30 ms, RPC by reference 1.6 ms, pyo3 `bytes` 0.48 ms, pyo3 `memoryview` 0.18 ms.

**Reading.**

- **Inline JSON over RPC grows superlinearly worse:** 3x at 100 KB, 9–10x at 1 MB, 15–17x at 10 MB.
- **By reference, in-process and RPC are equivalent from 1 MB up,** because reading the file dominates. In-process zero-copy is 6–25x faster still, because it skips the read.
- **The remaining gap:** at 1–10 MB it is one file read of about 1.5 ms. A fetch or render costs far more than that per page; a shared-memory mapping would close it if it ever mattered (not built).

## S5: one SQLite file, several SDK processes (1,000 pages of 1 KB per process, concurrency 8)

The table shows medians over 5 blocks, without the inspector.

| SDK | Processes | Embedded: rows/s, commit p99 (worst process), lock wait summed | Daemon: rows/s, commit p99, lock wait |
| --- | ---: | --- | --- |
| Node | 1 | 1,439, 62 ms, 7 ms | 2,532, 4.9 ms, 5 ms |
| Node | 4 | 3,313, 89 ms, 1.3 s | 2,889, 20 ms, 24 ms |
| Node | 8 | 3,620, 189 ms, **10.2 s** | 2,673, 79 ms, 58 ms |
| Python | 4 | 2,046, 118 ms, 2.0 s | 1,401, 26 ms, 44 ms |
| Python | 8 | 3,280, 134 ms, **8.2 s** | 2,590, 45 ms, 44 ms |

Paired ratios, daemon ÷ embedded (5 blocks):

| Processes | Throughput, Node | Throughput, Python | Commit p99, Node | Commit p99, Python |
| ---: | --- | --- | --- | --- |
| 1 | 1.34 (0.68–23.4) | 1.10 (0.89–3.32) | 0.09 (0.01–3.75) | 0.57 (0.17–2.41) |
| 2 | 1.08 (0.71–2.77) | 1.79 (0.49–8.95) | 0.31 (0.04–1.20) | 0.45 (0.05–1.25) |
| 4 | 0.75 (0.62–1.58) | 1.02 (0.22–2.06) | **0.30 (0.07–0.41)** | **0.54 (0.09–0.76)** |
| 8 | 0.91 (0.30–1.61) | 0.79 (0.71–1.10) | **0.17 (0.10–0.88)** | **0.26 (0.07–0.55)** |

- **Embedded works at N ≤ 8 on this Mac.** There were no `SQLITE_BUSY` errors escaping `busy_timeout` 5000, no lost rows and `integrity_check` was ok in all 160 runs. Total throughput is the same within noise: every throughput CI spans 1, and both topologies sustain about 1.4–4k rows/s.
- **Where it shows:**
  - **Tail latency and lock waiting:** at 4–8 processes the daemon's commit p99 is 2–6x lower, with CIs below 1. Embedded engines together spend 8–10 s waiting on `BEGIN IMMEDIATE` at 8 processes.
  - **Commit count:** about 2.8k commits for embedded vs 0.7–1.1k for the daemon. The daemon group-commits *across* clients, so it pays fewer fsyncs. That matters more on Linux, where fsync is not cheap; this is an inference and was not measured.
- **Read-only inspector** (polling `GROUP BY status` every 50 ms): no measurable effect on throughput; all CIs span 1. Its query p99 was 1–29 ms. The WAL stayed around 4 MB, the size at which auto-checkpoint runs, in both topologies.

## Feel

| | napi-rs (Node) | PyO3 (Python) | RPC (engined + both SDKs) |
| --- | --- | --- | --- |
| Glue lines (non-blank, non-comment; engine-core shared, 499) | 79 Rust + 35 TS = **114** | 179 Rust + 47 Py = **226** | 239 Rust (about 50 of them `inspect`/`check`) + 126 TS + 115 Py = **480** for both languages |
| Clean release build (one unpaired sample, load 400–800) | 59 s | 72 s (maturin) | 268 s (built first; probably load and order, not the crate) |
| Artifact | 3.1 MB `.node` (2.8 MB stripped) | 1.5 MB abi3 wheel (3.1 MB `.so`), one wheel for CPython ≥ 3.11 | 3.4 MB binary (3.0 MB stripped), shared by both SDKs |
| Handler throws: what reaches the engine | `GenericFailure, boom: …`: **message only, no JS stack**. A sync throw gives `PendingException, sync boom` | Async: full Python traceback (formatted on the Rust side through `traceback.format_exception`). Sync (non-async handler): `ValueError: sync boom` only | Full stack trace in both languages, because the SDK puts `err.stack` / `traceback.format_exc()` into the JSON-RPC `error.data` |
| Version skew | Compiled together; Node-API level and the `.node` per platform are the only axes | abi3 floor (3.11 here, needed for the buffer protocol); free-threaded `cp314t` is not covered by abi3 | Explicit: `initialize` with protocol `2.0` gets `-32002 protocol version mismatch: client 2.0, engine 1.0` in both SDKs |

What was awkward:

- **napi-rs**
  - `call_async` routes a sync JS throw through `napi_fatal_exception`, which **kills the process**, according to napi 3.14.2's own doc comment. This was read, not triggered; `call_async_catch` is needed.
  - `#[napi(object)]` camel-cases fields by default, so the in-process page shape silently differed from the RPC wire shape until `js_name` was added. Two bindings mean two contracts to keep in sync.
  - A `ThreadsafeFunction` keeps Node alive while a run holds it; it was released correctly when the run resolved.
  - JS promises cannot be cancelled (S2b).
- **PyO3**
  - **Toolchain:** the macOS 27 beta linker (ld-27037) produced a dylib whose LINKEDIT string pool was 4-byte aligned, after an odd count of indirect symbols, and dyld refused to load it ("mis-aligned LINKEDIT string pool"). It was reproducible with and without LTO and with a 14.0 deployment target. The workaround was to link the PyO3 crate with `rust-lld` against the 15.5 SDK (`build.sh`). The napi addon linked fine by luck of layout. This is a beta-toolchain bug, but it is exactly the kind of per-platform native-build failure that only in-process extensions hit.
  - **Cancellation:** `pyo3-async-runtimes` does not cancel the Python task when its Rust future is dropped (S2b).
  - **Threads vs the event loop:** the natural "call the Python callback" path runs user code on a Tokio thread, not on the event loop. The asyncio-safe path (`call_soon_threadsafe`) is unbounded.
  - **Sync API:** a sync Python API (no asyncio) would need its own loop thread; only an async API was built.
  - **Sync throws:** errors from a non-async handler lose the traceback unless they are formatted explicitly.
- **RPC**
  - **More code:** a framing layer, a pending-request map, cancel propagation in both directions and disconnect cleanup, written once per SDK language.
  - **Large lines:** 10 MB inline pages make single JSON lines of about 10 MB. The asyncio `StreamReader` limit had to be raised to 256 MB, which is another argument for by-reference.
  - **Distinguishing failure modes:** transport loss must be told apart from handler errors (the S3 bug).
  - **Flow control for async consumers** needs protocol support (S1).
  - **What comes for free:** stack traces, cancellation semantics, version negotiation, crash isolation and lease release on disconnect all fall out of having a protocol.

## What this suggests for the binding decision (evidence, not a decision)

- **Supports "engine out of the SDK process":**
  - **Crash recovery:** only the out-of-process topologies released a crashed client's leases (0.2–0.4 ms after EOF), and a daemon kept serving other clients through `exit` and `abort` (S3).
  - **Contention:** at 4–8 SDK processes on one database, a daemon cut commit p99 2–6x and avoided seconds of lock waiting (S5).
  - **Errors and versions:** full stack traces and explicit version errors come almost free over RPC. napi lost the JS stack by default.
- **Weakens or nuances it:**
  - **SQLite safety does not need a separate process.** In-process crashes left the WAL database consistent and lost no acknowledged page (S3).
  - **Embedded multi-process works on this Mac.** It had no `SQLITE_BUSY` errors and no throughput loss at N ≤ 8 (S5). The cost is tail latency, not correctness.
  - **The boundary does not help a CPU-bound or blocking handler.** User-side progress stalls the same way in both (S2c).
  - **Cancellation latency is equal** (S2a).
- **Supports "pages by reference":**
  - **Inline JSON costs 3x, 9–10x and 15–17x** at 100 KB, 1 MB and 10 MB (S4).
  - **By reference, RPC matches in-process** by reference from 1 MB up.
  - **Counterpoint:** in-process zero-copy (napi `Buffer`, PyO3 `memoryview`) is a further 6–25x faster at the delivery step, about 1.5 ms per 10 MB page. That is real, but small next to a fetch or render.
- **Supports "coarse / batched operations", strongly:**
  - **Batching events** was the largest lever in every binding (S1).
  - **Bounded backpressure** at consumer speed needed a bounded engine queue *plus* batched delivery whatever the binding.
  - **The native bindings did not get cheaper backpressure;** their defaults buffer without bound too.
- **Open, not measured here:**
  - Clean-machine install.
  - Linux fsync costs; S5 would likely favour the daemon's cross-client group commit more.
  - Windows.
  - A credit or ack mechanism for async event consumers over RPC.
  - Lease expiry for in-process crashes.
