# Shipping TypeScript and Python SDKs over a Rust engine

Research of October 10, 2026, for ticket [#63](https://github.com/Jbmanllr/automan/issues/63) (map [#61](https://github.com/Jbmanllr/automan/issues/61)). It compares three ways to put TypeScript and Python SDKs on top of a Rust/Tokio engine: in-process native bindings, RPC to a sidecar or daemon binary, and WASM. Facts come from official docs, source code, package registries and release notes, linked inline. Package sizes and platform lists were read from the npm registry and PyPI JSON APIs on October 10, 2026. A local microbenchmark was also run (§3). It is labelled **measured**; vendor figures are labelled **claimed**. Nothing here is an adopted decision.

## 1. Summary

- **All three families ship in production, with different trade-offs.**
  - **In-process native bindings:** Rspack, lightningcss, SWC and nodejs-polars use napi-rs; Polars, pydantic-core and tokenizers use PyO3. Temporal's Rust core uses Neon for Node and PyO3 for Python.
  - **Sidecar or daemon binaries:** Playwright's driver (Python, Java, .NET), esbuild, Browser Use's `sdk-server` and agent-browser.
  - **WASM:** Restate's TypeScript SDK, Prisma's query compiler and Biome's JS API. Each leaves I/O to the host: Restate's core states it "does not perform I/O", and Prisma's compiler returns plans that TypeScript executes.
- **Callback cost is not the deciding factor for a per-page handler (measured, §3 and §3.1).** On a heavily loaded M3 Max, one engine-to-handler call measured as follows:
  - **Small payload (64 B):** in-process is about **2x faster**, but the gap is tens of microseconds. The paired re-run (§3.1, 8 blocks) gives an in-process/RPC time ratio of **0.47** for Node (95% CI 0.38–0.65; medians 26 µs vs 54 µs) and **0.64** for Python (CI 0.53–0.84; 99 µs vs 120 µs). The first, unpaired passes had suggested overlapping ranges; pairing removes the load drift that hid the difference.
  - **Large payloads:** moving the HTML costs more than the call. Inline JSON over a pipe took 0.57–0.83 ms at 100 KB and 4.2–7.6 ms at 1 MB. In-process took 0.14–0.40 ms and 0.6–3.3 ms.
  - **Scale:** all of this is small next to a page fetch or render, and large pages should cross the boundary by reference.
  - **The real multiplier is a chatty API,** such as one call per `click`. That is why esbuild, Rspack and Lightning CSS all document ways to reduce JS callbacks.
- **The engine's state is a stronger constraint than call cost.**
  - The engine owns a durable journal in SQLite with a single writer, plus Chromium processes and leases.
  - **In-process binding:** that state lives inside every user process. Several Node or Python processes on one machine would each embed an engine and compete for the same SQLite write lock. A native crash in user code also takes the engine down.
  - **Daemon:** one process owns the database, and clients come and go.
  - Temporal can be in-process because the durable state lives on the Temporal server, not in the SDK process.
- **One engine binary keeps the release matrix smallest.**
  - **One binary per OS, CPU and libc** serves the TS SDK, the Python SDK, the CLI and the MCP server. It ships through npm `optionalDependencies` (esbuild, Biome) and `py3-none-<platform>` wheels (uv, ruff, browser-use-core).
  - **PyO3 wheels add a Python-ABI axis:** pydantic-core 2.50.0 ships **136 wheels**. Native Node addons do not, because Node-API is ABI-stable across Node versions.
- **WASM does not fit the engine.** It works for cores without I/O. Automan's engine needs sockets, child processes (Chromium), threads and files. esbuild documents its WASM build as often about 10x slower than native. WASM stays an option for pure helpers, such as extractors in the browser.
- **Direction to test, not a decision:**
  - A Rust engine binary that can run as a short-lived sidecar or a shared local daemon.
  - Thin TS and Python SDKs over one versioned, bidirectional JSON-RPC contract. The engine calls user handlers with server-to-client requests, as Playwright route handlers and Stagehand's `llm.generate` already do.
  - The Rust crate stays embeddable. Native bindings stay a later option for hot, I/O-free helpers.
  - The open items are in §6.

## 2. Comparison

Legend: **M** = measured here (§3), **R** = read from registry metadata, **D** = vendor docs or source.

| Criterion | In-process: napi-rs (Node) | In-process: PyO3 / maturin (Python) | RPC to sidecar or daemon binary | WASM |
| --- | --- | --- | --- | --- |
| Install friction | No compiler or install script: npm picks one optional package by `os`/`cpu`/`libc` ([napi-rs release model](https://napi.rs/docs/deep-dive/release)). Known failures: the lockfile bug that drops other platforms' optional deps ([npm/cli#4828](https://github.com/npm/cli/issues/4828), 307 comments, closed 2025-04), `--omit=optional`, and copying `node_modules` across OSes ([esbuild](https://esbuild.github.io/getting-started/)) | A matching wheel installs with no toolchain. Otherwise pip falls back to the sdist, which needs Rust ([maturin](https://www.maturin.rs/distribution.html)). Example: restate-sdk publishes no Windows wheel **R** | Same npm mechanism for the binary (esbuild, Biome), or one package carrying every binary plus a `postinstall` (agent-browser **R**). Python: `py3-none-<platform>` wheels with no Python-ABI axis (uv, ruff, browser-use-core **R**). A missing platform has no fallback when no sdist exists (playwright, browser-use-core **R**) | One artifact for every OS. napi-rs marks its WASI package `cpu: ["wasm32"]`, so it installs only on request ([napi-rs WASI](https://napi.rs/docs/concepts/webassembly)) |
| Prebuilt artifacts per release | One `.node` per OS/CPU/libc, valid on every Node with that Node-API level ([support](https://napi.rs/docs/more/support-compatibility)). Rspack ships 14, SWC 12, lightningcss 11 **R** | Per OS/CPU/libc **times** Python ABI. abi3 collapses versions (Temporal 7 wheels, Polars runtime 8 **R**). Free-threaded builds cannot load abi3 and need `cp314t` wheels until `abi3t` ([PyO3](https://pyo3.rs/main/building-and-distribution.html)). pydantic-core ships 136 wheels **R** | One binary per OS/CPU/libc, shared by every SDK and the CLI. esbuild ships 26, uv 18, ruff 17, browser-use-core 5, agent-browser 7 **R** | One `.wasm` (Rspack `binding-wasm32-wasi`, `@biomejs/wasm-nodejs` **R**) |
| Size per platform | Rspack binding 39–70 MB, lightningcss 8–10 MB, nodejs-polars 100–141 MB (unpacked) **R** | Polars runtime 48–55 MB, Temporal 14–15 MB, pydantic-core 1.5–2.5 MB (wheel) **R** | agent-browser 15.6–18.3 MB per binary; browser-use-core wheels 22–27 MB; Playwright Python 34–48 MB because each wheel bundles Node **R** | Biome WASM 47 MB unpacked. Prisma's query compiler is 1.6 MB versus 14 MB for its Rust engine ([Prisma](https://www.prisma.io/blog/rust-to-typescript-update-boosting-prisma-orm-performance), **claimed**) |
| Plain call overhead | 0.17–0.62 µs per sync call (JS baseline 2–11 ns). Passing a `Buffer` costs 0.2–1.1 µs at any size. A 100 KB string costs 0.20–0.43 ms **M** | 46–108 ns per sync call (Python baseline 32–129 ns) **M** | One stdio JSON-RPC round trip: 31–150 µs with a 64 B payload, plus JSON encoding of larger payloads **M** | Prisma attributes up to 3.4x faster queries to removing Rust↔JS serialization (**claimed**, [benchmark table](https://www.prisma.io/blog/rust-to-typescript-update-boosting-prisma-orm-performance)) |
| Engine → user handler call (p50) | `ThreadsafeFunction` from a Tokio task, awaiting the returned Promise. 26–40 µs at 64 B, 0.26–0.29 ms at 100 KB, 1.8–3.3 ms at 1 MB **M** | `into_future` on the handler's coroutine. 83–119 µs at 64 B, 0.14–0.40 ms at 100 KB, 0.6–1.7 ms at 1 MB **M** | Server-to-client request over the pipe. Node host: 31–150 µs, 0.60–0.79 ms, 5.3–5.8 ms. Python host: 68–118 µs, 0.57–0.83 ms, 4.2–7.6 ms **M** | Not applicable to an I/O engine |
| Async interop | napi-rs runs `async fn` on its own Tokio runtime and returns a Promise. JS can only be called on the JS thread, through a queued `ThreadsafeFunction` ([TSFN](https://napi.rs/docs/concepts/threadsafe-function), [async fn](https://napi.rs/docs/concepts/async-fn)) | Python keeps the main thread; Rust runtimes run in the background. Calling coroutines needs the event-loop locals ([pyo3-async-runtimes](https://github.com/PyO3/pyo3-async-runtimes)). Native `async fn` is still experimental ([PyO3](https://pyo3.rs/main/async-await.html)) | Each side keeps its own loop; the boundary is a socket. Cancellation and backpressure have to be protocol messages | The engine has to give up Tokio, or register a thread-free executor ([napi-rs async runtime](https://napi.rs/docs/concepts/async-runtime)) |
| Crash isolation | Shared process: an uncaught panic in a sync callback can end the process; `catch_unwind` is opt-in and partial ([napi-rs errors](https://github.com/napi-rs/website/blob/main/pages/en/docs/concepts/error-handling.md)). An OOM or segfault in user code kills the engine | Same. Panics surface as [`PanicException`](https://docs.rs/pyo3/latest/pyo3/panic/struct.PanicException.html), but aborts and segfaults kill the interpreter | Separate processes. The client sees EOF and fails pending calls (Browser Use client code); the engine can mark the attempt failed and respawn the handler host | Sandboxed memory, but the same process |
| Debugging | Mixed native/JS stacks; needs a native debugger and debug symbols | `rust-lldb`/`rust-gdb` attached to `python` ([PyO3 debugging](https://pyo3.rs/main/debugging.html)) | Each process debugs with its own tools. The wire can be logged: Playwright prints every message when `DEBUGP` is set ([transport](https://github.com/microsoft/playwright-python/blob/main/playwright/_impl/_transport.py)) | Separate debug `.wasm`; limited stacks |
| Other JS runtimes | Bun is best effort and Deno is not in napi-rs CI ([support](https://napi.rs/docs/more/support-compatibility)). A workerd collaborator says native addons are "likely never to be supported" on Workers ([workerd #1905](https://github.com/cloudflare/workerd/discussions/1905)) | CPython; PyPy and GraalPy need their own wheels (pydantic-core ships them **R**) | Any runtime that can open a socket or spawn a process. Workers cannot spawn, but could reach a remote engine over WebSocket | Workers, browsers and Deno ([Prisma](https://www.prisma.io/blog/rust-to-typescript-update-boosting-prisma-orm-performance)) |
| Hosted service: user handlers | The engine runs inside the user's process image, so the cloud would have to run the engine inside the untrusted container (inference) | Same | User code in an isolated handler host (Node/Python image plus the package) that talks to an engine worker over the same contract as locally. Restate's server already calls handlers over HTTP ([Restate services](https://docs.restate.dev/concepts/services)) | Only for I/O-free helpers |
| Several processes on one SQLite | Each process embeds an engine and opens the database. SQLite serializes writers ([FAQ](https://www.sqlite.org/faq.html)). Two SQLite copies in one process can break POSIX locks ([howtocorrupt §2.3](https://www.sqlite.org/howtocorrupt.html)) | Same, plus `fork` with live threads or connections ([howtocorrupt §2.7](https://www.sqlite.org/howtocorrupt.html), [Polars](https://docs.pola.rs/user-guide/misc/multiprocessing/)) | One daemon owns the database and stays the only writer; clients only send RPCs | Not applicable |

## 3. Local measurement: binding versus stdio JSON-RPC

**Why.** The R&D proposed JSON-RPC over stdio or WebSocket and said its overhead "will be measured" ([ADR-001 draft](../adr/0001-runtime-language.md), [architecture §8.1](../proposals/architecture.md)). No lab report measured it. This is a first microbenchmark of the path that matters for a Crawlee-style engine: **the Rust engine calls a user `async` handler once per page with the page HTML and awaits a small result.**

**Setup (measured).**

- **Host:** Apple M3 Max (14 cores), macOS, Node 24.19.0, CPython 3.13.11, Rust 1.93.0. Built with napi 3.14.2, napi-derive 3.6.12, PyO3 0.29.3, pyo3-async-runtimes 0.29.0 and Tokio 1.53.2, in release mode with LTO.
- **Load:** the host was **heavily loaded by unrelated work**, with a load average of 290–860 during the runs (`uptime` recorded before and after each block). Medians are therefore indicative and tails (p99) are unusable: they reached 0.1–90 ms in every variant. A quiet Linux reference host is needed before any figure becomes a gate.
- **Payload:** synthetic HTML with quotes, `&` and non-ASCII characters, at 64 B, 100 KB and 1 MB. The handler returns `{"n": len}`.
- **Variants:**
  - **In-process Node (napi-rs).** A `#[napi] async fn` runs a Tokio task that calls the JS handler through a `ThreadsafeFunction<String, Promise<String>>`. It awaits both `call_async` and the returned Promise.
  - **In-process Python (PyO3).** A Tokio task attaches to the interpreter, calls the `async def` handler and converts the coroutine with `into_future_with_locals`.
  - **stdio JSON-RPC.** A Rust Tokio "engine" spawns `node handler.mjs` or `python handler.py` and sends newline-delimited JSON-RPC 2.0 requests with the HTML in `params`. The host parses each line, awaits the handler and writes the response line. This is the same framing Browser Use's `sdk-server` uses.
- **Repeats and code:** sequential calls after a 500-call warmup, plus 16 concurrent callers for throughput.
  - Three full passes were run back to back between 08:31 and 08:45 UTC. The napi-rs variant also had one standalone pass, at 08:16 UTC.
  - An earlier stdio pass was discarded: its Node host re-concatenated the input buffer on every chunk, which is quadratic for 1 MB lines.
  - The harness lives in the session scratchpad and is not committed; it can be rebuilt from this description.

**Results (p50 as min–max across passes, measured).**

| Path | 64 B | 100 KB | 1 MB | 16 concurrent callers, 64 B |
| --- | --- | --- | --- | --- |
| Baseline: JS `await` of an async function | 0.25–0.29 µs | — | — | — |
| Baseline: Python `await` of a coroutine | 1.5–1.9 µs | — | — | — |
| napi-rs: JS → Rust `async fn` (Tokio) → Promise | 19–41 µs | — | — | — |
| **napi-rs: Rust → JS handler → Promise** | **26–40 µs** | **0.26–0.29 ms** | **1.8–3.3 ms** | 48k–164k calls/s |
| PyO3: Python → Rust `async fn` → awaitable | 60–111 µs | — | — | — |
| **PyO3: Rust → Python handler → coroutine** | **83–119 µs** | **0.14–0.40 ms** | **0.61–1.7 ms** | not run |
| **stdio JSON-RPC: Rust → Node handler host** | **31–150 µs** | **0.60–0.79 ms** | **5.3–5.8 ms** | 148k–235k calls/s |
| **stdio JSON-RPC: Rust → Python handler host** | **68–118 µs** | **0.57–0.83 ms** | **4.2–7.6 ms** | 32k–46k calls/s |

Plain calls and startup (measured, ranges across passes):

- **Node:** a sync napi call cost 0.17–0.62 µs, against 2–11 ns for a JS function. Passing a `Buffer` cost 0.2–1.1 µs at any size. A JS string cost 0.20–0.43 ms at 100 KB and 1.6–2.5 ms at 1 MB, because it is transcoded to UTF-8.
- **Python:** a sync PyO3 call cost 46–108 ns, against 32–129 ns for a Python function. Passing `&str` cost about 0.1–0.2 µs at every size, but only because CPython caches the UTF-8 form of a string that is reused. A fresh string pays the conversion once.
- **Startup:** spawning the handler host and getting the first reply took 80–270 ms for Node and 76–103 ms for Python.

**Reading (inference from these numbers).**

1. **For small calls, the process boundary costs about 2x, or roughly 25–30 µs per call.** This is corrected by the paired re-run (§3.1); the unpaired passes above had overlapping ranges and suggested no measurable difference.
   - **Node:** paired ratio 0.47 (CI 0.38–0.65): 26 µs in-process against 54 µs over stdio.
   - **Python:** paired ratio 0.64 (CI 0.53–0.84): 99 µs against 120 µs.
   - **Why both are tens of microseconds:** both paths are dominated by waking another thread or process, so an in-process call is not free either.
   - **With 16 calls in flight at 64 B,** the two are equivalent (ratio 1.04, CI 0.48–3.11), because one pipe read carries many messages.
   - **Scale:** 28 µs per page is about 3% of one core at 1,000 pages per second. It matters for chatty APIs (item 3), not for one call per page.
2. **Large payloads are where the boundary costs.**
   - **Inline JSON over a pipe** cost about 2–4x the in-process path at 100 KB: 0.57–0.83 ms against 0.14–0.40 ms. At 1 MB it cost 4.2–7.6 ms against 0.6–3.3 ms.
   - **In-process is not free either:** a 1 MB string still costs milliseconds to transcode.
   - **The fix is the same in both models:** pass page content by reference rather than inline. Candidates: an artifact id or file path the handler reads on demand, binary framing, or a `Buffer` in-process.
3. **Per-page handler cost is minor; chatty APIs are not.** One call per page is small next to network and render time. A handler that makes many fine-grained calls back into the engine, such as one per `click` or `evaluate`, pays one round trip each. That is the case for batching and coarse operations: Stagehand's `experimentalBatch`, Rspack's loader composition (§4).

### 3.1 Paired re-run (2026-10-10)

The passes above were unpaired, so load drift between them could hide a difference. This re-run uses the lab's paired-block design. There are **8 blocks**, and each runs the four runners (napi, PyO3, Node stdio, Python stdio) in a **fresh random order**, recording other processes' CPU before each runner. For each block it computes the ratio *in-process time / RPC time* (below 1 means in-process is faster), then reports the median across blocks with a bootstrap 95% CI (10,000 resamples). The code is unchanged and committed next to this note in [`2026-10-10-sdk-binding-bench/`](2026-10-10-sdk-binding-bench/), with the raw per-block results.

- **Host:** M3 Max (14 cores), macOS. The host was very busy: other processes used 405–1,294% CPU before each runner, and the load average was 350–900 (iOS Simulator, Xcode, two VMs). This host would be class `local-dev` in the lab, so absolute durations are indicative only. The paired ratios cancel most of the shared load drift, but they do not replace a quiet Linux reference host before anything becomes a gate.

| Comparison (p50 per call, or throughput) | In-process (median) | stdio JSON-RPC (median) | Ratio in-process/RPC | 95% CI |
| --- | ---: | ---: | ---: | --- |
| Node handler call, 64 B | 26 µs | 54 µs | **0.47** | 0.38–0.65 |
| Python handler call, 64 B | 99 µs | 120 µs | **0.64** | 0.53–0.84 |
| Node handler call, 100 KB inline | 245 µs | 685 µs | **0.31** | 0.19–0.38 |
| Python handler call, 100 KB inline | 155 µs | 497 µs | **0.22** | 0.08–0.46 |
| Node handler call, 1 MB inline | 1.93 ms | 5.83 ms | **0.29** | 0.21–0.49 |
| Python handler call, 1 MB inline | 0.62 ms | 9.44 ms | **0.08** | 0.03–0.15 |
| Node, 16 concurrent handlers, 64 B | 133k calls/s | 120k calls/s | 1.04 (equivalent) | 0.48–3.11 |
| Node, 16 concurrent handlers, 100 KB inline | 5.6k calls/s | 1.4k calls/s | **0.31** | 0.22–0.44 |

Sidecar spawn to first reply: median 73 ms for Node and 198 ms for Python over the 8 blocks.

**What changes:**
- **Small calls:** the boundary costs about 2x, or 25–30 µs per call. That corrects the "nothing measurable" reading of the unpaired passes.
- **Inline pages over a pipe:** they cost 3–12x more than in-process, so passing pages by reference is a requirement, not an optimization. The RPC numbers above send the HTML inline in JSON; a by-reference design was not measured.
- **What stays:** the conclusions of §1 and §6 hold. 28 µs per page call is small next to fetch and render time. The decisive constraints remain the engine's state (single SQLite writer), crash isolation and the release matrix. For a sidecar or daemon design, the paired data adds two firm requirements: **pages by reference** and **coarse or batched operations** instead of chatty per-action calls.

## 4. Case studies

### Temporal: a Rust core linked in-process, with inverted control

- **How the core is linked:** the Rust Core SDK is shared by the TypeScript, Python, .NET and Ruby SDKs. It uses Neon for Node, PyO3 for Python and C bindings for .NET ([ARCHITECTURE.md](https://github.com/temporalio/sdk-rust/blob/master/ARCHITECTURE.md)).
- **Who calls whom:** the language layer **polls the core** for activations and tasks, runs user workflow and activity code in its own runtime, and pushes completions back. The language side drives the loop; the core does not invoke user functions. The messages are protobuf.
- **Why in-process:** the team chose it so users "simply deploy one binary". They name an IPC sidecar as an alternative that "probably fail[s]" their ease-of-use goal. They kept protobuf so they could move out-of-process later ([Temporal blog](https://temporal.io/blog/why-rust-powers-core-sdk)).
- **Why it works for Temporal:** the durable state lives on the Temporal server, so a crashed worker loses no history.
- **Packaging (R):**
  - `@temporalio/core-bridge` 1.24.0 is one npm package of 152 MB unpacked. It holds five `index.node` files of 26–31 MB: macOS arm64/x64, Linux glibc arm64/x64 and Windows x64.
  - `temporalio` 1.34.0 ships 7 abi3 wheels of 14–15 MB.

### Restate: one sans-I/O core, compiled two ways, with handlers served over HTTP

- **One core, two bindings:** `sdk-shared-core` serves the TypeScript, Python, Go, Ruby and Rust SDKs ([repo](https://github.com/restatedev/sdk-shared-core)).
  - **TypeScript:** the core is compiled with `wasm-bindgen` and inlined into the package ([bindings Cargo.toml](https://github.com/restatedev/sdk-typescript/blob/main/sdk-shared-core-wasm-bindings/Cargo.toml)).
  - **Python:** it is a PyO3 extension built with maturin ([sdk-python Cargo.toml](https://github.com/restatedev/sdk-python/blob/main/Cargo.toml)).
- **Why WASM fits:** the integration guide says the core "is single-threaded and does not perform I/O". The SDK owns the HTTP transport and runs user code ([sdk-integration.md](https://github.com/restatedev/sdk-shared-core/blob/main/docs/sdk-integration.md)). That property is what makes a WASM build practical (inference), and the TS SDK also has a Cloudflare Workers package ([packages](https://github.com/restatedev/sdk-typescript/tree/main/packages/libs)).
- **Handlers are reached over HTTP:** the Rust server "sits in front of your services, similar to a reverse proxy". Handlers are deployed wherever the user wants and invoked by the server ([concepts](https://docs.restate.dev/concepts/services)).
- **Packaging (R):** `restate-sdk` 1.0.5 ships 39 wheels for CPython 3.10–3.14, 3.14t and PyPy, but none for Windows.

### Prisma: sidecar binary, then Node-API library, then TypeScript plus WASM

- **Three architectures in turn:** until Prisma 3.x the only engine was a sidecar binary. From 2.20 a Node-API library engine was available behind a preview flag, and it then became the default. The docs say the library "reduces the communication overhead" ([engines](https://www.prisma.io/docs/orm/v6/more/internals/engines.md)).
- **Then Rust was removed from the client:** a TypeScript executor plus a WASM query compiler replaced the Rust engine. It was production-ready in 6.16 and became the default in 7 ([Prisma 7](https://www.prisma.io/blog/announcing-prisma-orm-7-0-0)).
- **Stated reasons** ([performance update](https://www.prisma.io/blog/rust-to-typescript-update-boosting-prisma-orm-performance)):
  - "the cost of serializing data between Rust and TypeScript is very high".
  - Strict networks that cannot fetch the binary.
  - Production and development environments that end up with different binaries.
  - Lambda and Workers limits.
- **Claimed results:** 14 MB down to 1.6 MB, and up to 3.4x faster on 25,000-row reads, in Prisma's own benchmark repo (same source).
- **Lesson (inference):** a thin engine that hands large results to JS loses to serialization. Prisma's engine returned query data to the client, whereas Automan's engine does the I/O itself and can return extracted records.

### Playwright: one Node driver, many client languages

- **Spawning and framing:** the Python client starts the bundled Node driver (`node cli.js run-driver`) and exchanges JSON messages framed with a 4-byte little-endian length ([transport](https://github.com/microsoft/playwright-python/blob/main/playwright/_impl/_transport.py), [driver path](https://github.com/microsoft/playwright-python/blob/main/playwright/_impl/_driver.py)).
- **Callbacks:** user callbacks run in the client. A route handler, for example, is invoked when the driver emits a `route` event on the channel, and the handler answers with further protocol calls ([browser context](https://github.com/microsoft/playwright-python/blob/main/playwright/_impl/_browser_context.py)).
- **Sync API:** it relies on `greenlet` (`requires_dist` **R**).
- **Packaging (R):** `playwright` 1.63.0 ships 8 wheels of 34.5–48.2 MB with no sdist. The size comes from the bundled Node runtime.
- **Lesson:** a sidecar in another language is viable at scale, but carrying a runtime is costly. A Rust binary avoids that cost.

### Stagehand v4: bidirectional JSON-RPC to an in-browser runtime

- **Where the runtime runs:** in a Chrome extension. The SDKs talk to it over bidirectional JSON-RPC carried on the CDP connection: the TS SDK builds its `RPCClient` on `browser.cdpClient` ([stagehand.ts](https://github.com/browserbase/stagehand/blob/main/packages/sdk-ts/src/stagehand.ts)).
- **Schemas and models:** Zod schemas produce `stagehand.v4.json`, which generates the Python and Go models.
- **Versioning:** SDK, extension and protocol versions are independent. Only `protocolVersion` gates compatibility, with an explicit SemVer rule ([protocol README](https://github.com/browserbase/stagehand/blob/main/packages/protocol/README.md)).
- **Server-to-client requests:** the runtime can ask the SDK for work. When the user supplies a client-side model, the Python SDK registers an `llm.generate` request handler, so the runtime's model calls are answered inside the user's process ([stagehand.py](https://github.com/browserbase/stagehand/blob/main/packages/sdk-python/src/stagehand/stagehand.py)).
- **Batching:** `experimentalBatch` ships a JS callback's source text (`Function.prototype.toString`) to run next to the page ([stagehand.ts](https://github.com/browserbase/stagehand/blob/main/packages/sdk-ts/src/stagehand.ts)). That only works for JS that is valid in the browser.
- **Packaging (R):** `stagehand` 4.2.0 on PyPI is a single 0.5 MB pure-Python wheel.

### agent-browser: a Rust CLI and a Rust daemon

- **Transport:** the CLI connects to a per-session daemon over a Unix socket, or TCP on loopback on Windows, using newline-delimited JSON ([connection.rs](https://github.com/vercel-labs/agent-browser/blob/main/cli/src/connection.rs)).
- **Lifecycle:** the daemon starts on first use and exits after one idle hour. `doctor` cleans stale `.sock` and `.pid` files ([README, Architecture](https://github.com/vercel-labs/agent-browser#architecture)).
- **Extensions:** plugins run **out of process** over a stdio JSON protocol (`agent-browser.plugin.v1`) and declare capabilities.
- **Packaging (R):** npm `agent-browser` 0.39.0 is one 119 MB package. It holds seven binaries of 15.6–18.3 MB (macOS arm64/x64, Linux glibc and musl arm64/x64, Windows x64) and selects one with a `postinstall` script. These facts come from `main` at `44af398`.

### Browser Use: a Rust runtime behind a Python SDK

- **Spawning:** the `browser-use` beta agent starts `browser-use-terminal sdk-server --transport stdio`.
- **Protocol:** JSON-RPC 2.0, one message per line. The client's stream limit defaults to 64 MB, and pending calls fail when the process exits ([service.py](https://github.com/browser-use/browser-use/blob/main/browser_use/beta/service.py)).
- **Distribution:** the binary ships in the `browser-use-core` wheels, smoke-tested in CI with a `runtime.ping` request ([wheel workflow](https://github.com/browser-use/terminal/blob/main/.github/workflows/browser-use-core-wheels.yml)).
- **Python called back from Rust:** the runtime keeps a persistent Python worker subprocess for its `python` tool ([REARCHITECTURE.md](https://github.com/browser-use/terminal/blob/main/REARCHITECTURE.md)).
- **Packaging (R):** 5 wheels of 22–27 MB, with no sdist.

### esbuild, Rspack, Lightning CSS, SWC and Biome: the cost of user callbacks

- **esbuild:** the JS API runs the Go binary as a child process ([getting started](https://esbuild.github.io/getting-started/)), and plugins are JS callbacks reached over that IPC. Its docs require a Go-side regex filter on every callback because "calling from esbuild's highly-parallel internals into single-threaded JavaScript code is expensive" ([plugins](https://esbuild.github.io/plugins/)). The WASM build "can sometimes be 10x slower" ([getting started](https://esbuild.github.io/getting-started/)).
- **Rspack:** in-process via napi-rs. It composes consecutive JS loaders into one round trip because "we must reduce the communication between Rust and JS as minimum as possible" ([loader architecture](https://v1.rspack.dev/contribute/architecture/rspack-loader)).
- **Lightning CSS:** JS visitors can make compilation "around 2x slower" ([transforms](https://lightningcss.dev/transforms.html)).
- **SWC loader:** function options cause "performance overhead as these functions must be called from Rust" ([builtin swc-loader](https://rspack.rs/guide/features/builtin-swc-loader)).
- **Biome:** ships its CLI as native binaries, 8 packages of 57–79 MB, and its JS API as WASM peer packages **R**.

### Polars and pydantic-core: the PyO3 matrix

- **pydantic-core 2.50.0 (R):** 136 wheels, with no abi3. They cover CPython 3.10–3.15 (about 15 wheels per version), free-threaded 3.14t/3.15t, PyPy 3.11, GraalPy and an Emscripten `wasm32` wheel.
- **Polars 2.0.0:** a pure-Python `polars` wheel plus `polars-runtime-32` binaries, 8 abi3 wheels of 48–55 MB **R**. The optional extras are `rt64` for more than 2^32 rows and `rtcompat` for CPUs without AVX ([installation](https://docs.pola.rs/user-guide/installation/)).
- **Fork hazard:** Polars documents that `fork` with its threads can deadlock and recommends `spawn` ([multiprocessing](https://docs.pola.rs/user-guide/misc/multiprocessing/)). Python 3.14 made `forkserver` the POSIX default for this class of problem ([multiprocessing](https://docs.python.org/3/library/multiprocessing.html)).

## 5. Single-writer SQLite when several processes embed the engine

- **SQLite allows it, one writer at a time:** several processes may open one database, but "only one process can be making changes to the database at any moment". Others get `SQLITE_BUSY` unless a busy handler waits ([FAQ](https://www.sqlite.org/faq.html)).
- **WAL limits:** WAL keeps a single writer and requires every process to be on the same host, because the wal-index is shared memory. It "does not work over a network filesystem" ([WAL](https://www.sqlite.org/wal.html)).
- **Hazards that apply to in-process embedding:**
  - **Two SQLite copies in one process:** POSIX advisory locks are dropped by any `close()` on the file, and SQLite's workaround keeps a per-library list of open files. With two SQLite copies linked into one process, one can silently drop the other's locks ([howtocorrupt §2.2–2.3](https://www.sqlite.org/howtocorrupt.html)). Node ships [`node:sqlite`](https://nodejs.org/api/sqlite.html) and CPython ships [`sqlite3`](https://docs.python.org/3/library/sqlite3.html). A native addon that bundles its own SQLite becomes a second copy as soon as user code opens the same file through either one.
  - **Fork:** do not carry an open connection across `fork()` (§2.7 of the same page). Engine threads started before a `fork` are not in the child, while their lock state is; Polars documents the resulting deadlock.
  - **Older SQLite:** the WAL-reset race (two connections writing or checkpointing at once) existed through 3.51.2 ([§8.1](https://www.sqlite.org/howtocorrupt.html)).
- **Consequence for Automan (inference).**
  - **In-process:** the engine is instantiated by each user process: each Node worker, each Python process under gunicorn or multiprocessing. Several engines would compete for the journal's write lock and would each believe they own the frontier and the leases.
  - **Fixes for that design:** one database per engine instance, or an engine-level leader lock with the others as clients. Both rebuild a daemon by other means.
  - **Daemon:** a single engine process keeps the SQLite file private and serializes writes in one writer task. This matches the R&D's single-writer model ([shared SQLite note](2026-10-08-workflow-and-shared-sqlite.md)).

## 6. Implications and open questions for Automan

**What the evidence supports (inference, to be confirmed by a decision ticket).**

1. **Keep the engine out of the SDK process by default.** Two shapes:
   - **Sidecar:** one SDK client spawns its own engine (Browser Use, Playwright).
   - **Daemon:** several clients attach over a local socket (agent-browser).

   Both use the same binary and the same protocol. This preserves the single-writer journal, isolates user crashes from the engine, and makes the local and hosted topologies identical.
2. **Make handler invocation a server-to-client request in a bidirectional protocol.**
   - **Existing practice:** Playwright's route events and Stagehand's `llm.generate` already work this way.
   - **What the contract needs:** cancellation and deadlines per request, backpressure (bounded in-flight handler calls), EOF semantics (fail or suspend the attempt), and protocol SemVer like Stagehand's.
   - **Large payloads:** page bodies go by reference, through an artifact handle or a lazy fetch, rather than inline JSON. If extraction runs in the engine, as R&D row 13 recommends (🟢, [rnd/README](../README.md)), many handlers receive records instead of raw HTML.
   - **One catalog:** the same contract can carry the single operation catalog that R&D row 16 wants served identically to the SDK, the CLI and MCP.
3. **Distribute one binary per platform through both ecosystems.**
   - **npm:** `optionalDependencies` with `os`/`cpu`/`libc` (esbuild, Biome, napi-rs layout).
   - **PyPI:** maturin `bin` wheels tagged `py3-none-<platform>` ([maturin bindings](https://www.maturin.rs/bindings.html)), as uv and ruff do.
   - **Matrix:** the same artifacts serve the CLI and MCP. A plausible starting matrix is macOS arm64/x64, Linux glibc and musl x64/arm64, and Windows x64. Chrome for Testing now publishes `linux-arm64` next to `linux64`, `mac-arm64`, `mac-x64`, `win32` and `win64` (its [download index](https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json), read October 10).
4. **Do not plan WASM for the engine.** Keep it for I/O-free helpers, such as extraction rules run in a browser or an edge runtime.
5. **Keep in-process bindings as an option, not the default.** They are worth it for a sync, I/O-free, hot helper such as an HTML-to-Markdown or selector engine. If one is added, the Temporal pattern (the host polls the core; protobuf or JSON messages) avoids Rust-to-JS callbacks.

**Open questions.**

- **Remeasure on the Linux reference host.** That covers p50 and p99, and Unix sockets against pipes against loopback TCP. Add a WebSocket transport for remote workers, plus the by-reference payload path. Today's figures come from a macOS host with a load average of 290–860.
- **Daemon lifecycle:**
  - auto-start and idle timeout (agent-browser uses one hour);
  - one daemon per user or per project;
  - version skew between an SDK and an already-running daemon;
  - discovery and permissions of the socket (§8.1 of the R&D already asks for a per-user credential).
- **Python sync API:** a greenlet bridge as in Playwright, or async only.
- **Windows:** named pipes against loopback TCP (agent-browser uses TCP), and whether Windows arm64 is in V1.
- **Hosted handler hosts:**
  - the image format (Apify builds each Actor from a Dockerfile on its base images, [Apify Docker images](https://docs.apify.com/sdk/js/docs/concepts/docker-images));
  - the trust boundary between the user container and the engine worker;
  - whether the cloud protocol is the local protocol over WebSocket or HTTP/2 as in Restate.
- **Local Console:** a resident daemon is the natural host for the locally served Console. That ties this choice to the map's "Console local architecture" item.
- **Measure the handler-to-engine nested call pattern** (`page.click` inside a handler) on real pages with CDP in the loop. Only that measurement shows whether batching is needed in V1.
