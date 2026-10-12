# JavaScript engines for the code-mode sandbox and Script Hooks

Research of **October 12, 2026** for ticket [#125](https://github.com/Jbmanllr/automan/issues/125) (map [#61](https://github.com/Jbmanllr/automan/issues/61)). Question: which JavaScript engine can run code mode (ADR 0011) and Script Hooks (ADR 0033) inside the engine binary, with no network or files of its own, only catalog calls, and time and memory limits? Candidates: QuickJS-NG through `rquickjs`, V8 through the `v8` crate (rusty_v8) or `deno_core`, and Boa. Others were checked briefly: the new `deno_core` QuickJS backend, QuickJS or SpiderMonkey compiled to WebAssembly and run in wasmtime, Nova, Hermes, JavaScriptCore and MicroQuickJS.

**Sources.** Crate archives from crates.io, read at the published version. The crates.io, GitHub, NVD, OSV and CISA KEV APIs. test262.fyi data, Boa's benchmark data and official project docs. Every claim links its source.

**Measured versus cited.** A microbenchmark was built and run for each engine, both on macOS arm64 and in a Linux musl container (§3). Measured facts are marked **M**; everything else is cited and marked **C**. The host was loaded by unrelated work (load average 10–118 during the runs), so timings are indicative medians, not gates. Like everything in `rnd/`, this note informs a decision; it does not make one.

---

## 1. Summary

- **Only QuickJS-NG (`rquickjs`) and V8 can enforce both limits.** Both enforced a memory limit and a wall-clock time limit in-process, and the runtime stayed usable afterwards **M**.
  - Boa 0.22 has no heap cap and cannot be stopped from outside **M**. A bounded script grew to 1.15 GB RSS over 30 s with nothing to stop it. Its only limits count loop iterations and recursion. The upstream interrupt issue has been open since 2023 **C**.
- **Neither engine's in-process limits are a complete boundary.**
  - **QuickJS-NG time limit.** The interrupt check runs once every 10,000 bytecode steps, and a long native builtin counts as one step. A 100 ms deadline therefore stopped `for(;;) big.fill(1)` only after **1.6 s** with 100,000 elements, and **16.6 s** with 1,000,000 elements **M**. JavaScript can also `catch` out-of-memory and continue: one probe caught it 156 times before the interrupt stopped it **M**.
  - **V8 memory limit.** Without a near-heap-limit callback, V8 aborts the whole process on out-of-memory (exit 133, "Fatal JavaScript out of memory") **M**.
  - **V8 time limit with the JIT on.** `terminate_execution` never stopped the loop from rusty_v8 issue #2088: on v8 152.2.0, macOS arm64, it was still running 5 s after termination **M**. `--jitless` and `--no-turbofan` both stop it in about 100 ms **M**.
  - **What this implies.** Either engine needs a backstop that does not depend on the engine itself, such as a watchdog that kills a helper process or thread.
- **On macOS, V8's JIT needs `allow-jit`, and `--jitless` does not** **M**.
  - Signed ad hoc with the hardened runtime and no entitlements, V8 in JIT mode aborts at isolate creation ("Failed to reserve virtual memory for CodeRange"). Adding `com.apple.security.cs.allow-jit` fixes it.
  - With `--jitless` the same binary runs every probe with no entitlement.
  - `rquickjs` (an interpreter) needs no entitlement.
- **Size and per-call cost favour QuickJS-NG; raw speed favours V8.** Stripped size added to a Tokio + serde_json binary on macOS arm64 **M**:

  | Engine | Size added |
  |---|---|
  | `rquickjs` | +1.1 MB |
  | Boa | +10.3 MB |
  | `v8` | +42.8 MB |
  | `deno_core` | +44.5 MB |

  Cost of calling a small hook 10,000 times with a JSON-ish argument converted in and out **M**:

  | Engine | Per call |
  |---|---|
  | V8, JIT | 0.96–1.20 µs |
  | V8, `--jitless` | 1.42–2.26 µs |
  | `rquickjs` | 4.8–5.5 µs |
  | Boa | 11.7–11.9 µs |

  Cold start of a runtime, a context and one hook call **M**:

  | Engine | Cold start |
  |---|---|
  | `rquickjs` | 62 µs |
  | Boa | 154 µs |
  | V8 isolate | 476 µs |
  | `deno_core` `JsRuntime` without a custom snapshot | 2.8 ms |

  Everything here is small next to a page fetch.
- **V8's security cost is volume and patch lag, not weak isolation.**
  - V8 is the most heavily attacked and fuzzed engine. NVD lists 137 V8 CVEs published so far in 2026, and CISA KEV lists 4 exploited in the wild in 2026 **C**.
  - The `v8` crate's last release was 2026-08-20, while Chrome now ships every two weeks. `deno_core` 0.412 still pins `v8` 150.4.0 **C**.
  - QuickJS-NG had 10 GitHub advisories in 2026, several of them memory corruption on out-of-memory paths **C**. On 2026-10-11 it stopped accepting private security reports and issuing CVEs **C**.
  - Boa is Rust with a GC built on `unsafe`, and has one GitHub advisory **C**.
- **Static musl builds.**
  - `rquickjs` ships bindings for all six V1 targets **M**, and its CI tests musl and Windows arm64 **C**.
  - It built and ran statically on aarch64 musl (native container) and on x86_64 musl (emulated) **M**.
  - Prebuilt musl static libraries for `v8` exist only since v150.2.0 (2026-07-16), and CI builds them without running tests **C**.
  - On aarch64 musl, `v8` and `deno_core` failed to link (`undefined reference to '__clear_cache'`) until `-C link-arg=-lgcc` was added. They then ran, and #2088 reproduced there too **M** (§3.6).
- **API stability.**
  - `deno_core` had 47 breaking releases in 12 months **C**.
  - The `v8` crate had 7 semver-major releases in 12 months **C**.
  - `rquickjs` went from 0.10 to 0.14 in the same year **C**. In 0.14 its `AsyncRuntime::idle()` prints failed jobs to stdout **M** (from source). That breaks the engine rule that only the transport writes stdout.
- **Something new.**
  - `deno_core` 0.412 now sits on `deno_v8`, an engine facade with a `quickjs` feature (QuickJS-NG through `v8x`) **C**.
  - It built and ran here: 7.3 MB, but 8.4 ms per cold `JsRuntime` **M**.
  - Deno labels the backend experimental **C**.

## 2. Comparison table

Legend: **M** measured here (§3), **C** cited.

| Criterion | QuickJS-NG via `rquickjs` | V8 via `v8` (rusty_v8) | V8 via `deno_core` | Boa (`boa_engine`) |
|---|---|---|---|---|
| Current version | `rquickjs` 0.14.0 (2026-09-18), vendoring QuickJS-NG 0.16.2+ (`QJS_VERSION` 0.16.2, NG commit `0fdea21`) **C/M** | 152.2.0 (2026-08-20), V8 15.2.124.1 **C/M** | 0.412.0 (2026-09-16) on `deno_v8` 0.4.0, which pins `v8` 150.4.0 **M** (lockfile) | 0.22.0 (2026-08-28) **C** |
| Licence | MIT (QuickJS-NG MIT) | MIT; V8 BSD-3-Clause plus bundled third-party notices | MIT | Unlicense OR MIT |
| Engine type | Bytecode interpreter in C | JIT (Ignition, Sparkplug, Maglev, TurboFan) or `--jitless` interpreter, in C++ | Same as V8 | Bytecode interpreter in Rust |
| Stripped size added (macOS arm64) | **+1.1 MB** **M** | **+42.8 MB** **M** | **+44.5 MB** **M** | **+10.3 MB** (default features include Temporal) **M** |
| Clean release build (loaded host) | 13 s vs 5 s baseline **M** | 33 s, including downloading the 40 MB gzipped prebuilt (158 MB `.a`) **M**; about 30 min from source **C** | 31 s **M** | 35 s **M** |
| Prebuilt for all 6 V1 targets | Bindings bundled for linux-musl x64/arm64, macOS x64/arm64, Windows msvc x64/arm64; needs a C compiler (`cc`) **C** | Static libraries for all six, musl since 2026-07-16 and build-only in CI **C**. aarch64 musl needs `-C link-arg=-lgcc` to link **M** | Same as `v8` | Pure Rust, any Rust target |
| Cold runtime + context + one hook call (p50) | **62 µs** **M** | **476 µs** isolate; 147 µs for a new context on an existing isolate **M** | **2.8 ms** `JsRuntime::new` without a custom snapshot **M** | **154 µs** **M** |
| Hook call with JSON in/out, 10k calls | **4.8–5.5 µs** **M** | **0.96–1.20 µs** JIT, **1.42–2.26 µs** jitless **M** | Same engine as `v8` (not run separately) | **11.7–11.9 µs** **M** |
| Empty function call | 0.019 µs **M** | 0.031–0.037 µs **M** | — | 0.055–0.061 µs **M** |
| Awaited async host op | 0.57 µs (Tokio `yield_now` inside) **M** | 0.20 µs (hand-rolled resolver loop, no Tokio) **M** | 13.6–16.1 µs with `yield_now`; 0.09 µs when the future is already ready **M** | 2.05 µs (Tokio `LocalSet`) **M** |
| Memory limit | `set_memory_limit`: throws, runtime reusable **M**. JavaScript can catch the out-of-memory error **M**. Works with `rust-alloc` despite the docs **M** | `heap_limits` plus a near-heap-limit callback that terminates, isolate reusable **M**. Without the callback the process aborts **M**. ArrayBuffer backing stores are outside the heap limit **C** | `add_near_heap_limit_callback` wrapper **C** | **None.** A thread-local GC with a growing threshold; 1.15 GB RSS reached unchecked **M** |
| Time limit | Interrupt handler, uncatchable `interrupted`, runtime reusable **M**. Polled every 10,000 steps, so long builtins overshoot: 100 ms became 1.6 s and 16.6 s **M** | `IsolateHandle::terminate_execution` from any thread, about 100 ms **M**. **Fails** on a TurboFan-optimized `fill` loop (#2088) **M**; jitless works **M** | Same as `v8` | **No interrupt.** Loop-iteration and recursion counters only **M**. `evaluate_async_with_budget` yields, but builtins ignore it **C**. RegExp has no backtracking limit **C** |
| macOS hardened runtime without `allow-jit` | Runs **M** | JIT aborts at startup; `--jitless` runs **M** | Aborts at startup (JIT by default) **M** | Runs (interpreter, not run signed here) |
| Async with Tokio | `AsyncRuntime`/`AsyncContext` and `Async(fn)` give Promises. `!Send` unless the experimental `parallel` feature is on **C** | Do-it-yourself: `PromiseResolver` plus `perform_microtask_checkpoint` **M** | `#[op2] async fn` becomes a Promise; `JsRuntime` is `!Send`, on a current-thread runtime **C** | `NativeFunction::from_async_fn` (the closure must be `Copy`), `JobExecutor`; `Context` is `!Send` **C/M** |
| test262 (test262.fyi, 2026-10-12) | 83.5% overall, mostly because Intl has 23 of 3,382 and there is no Temporal. `language`: 23,288 of 23,726 **C** | 97.67% (V8 15.7) **C** | Same | 95.59% (Boa's own report: 95.6% for v0.22) **C** |
| Throughput class (Boa's v8-v7 bench) | QuickJS (Bellard) 1,237; QuickJS-NG about 35% behind Bellard on another suite **C** | V8 jitless 2,515; V8 JIT about 40x QuickJS on another suite **C** | Same | 216 **C** |
| Security record | 26 NVD CVEs mentioning QuickJS since 2021 (10 in 2026). 10 NG GitHub advisories in 2026, 1 critical. NG stopped issuing CVEs on 2026-10-11. Not on OSS-Fuzz (Bellard's QuickJS is) **C** | V8 CVEs: 55 (2024), 46 (2025), 137 (2026 to date). KEV in-the-wild: 6, 4, 4. ClusterFuzz. V8 Sandbox off in rusty_v8 prebuilts **C** | Same, plus `deno_core` pins an older `v8` **C** | 1 GitHub advisory (high, 2024). `cargo-fuzz` targets in the repo, not on OSS-Fuzz **C** |
| Maintenance | `rquickjs`: 396 commits in 12 months (LLRT and Caido authors); NG releases about every 2 months **C** | Deno team; 7 majors in 12 months; no release since 2026-08-20 **C** | 47 breaking releases in 12 months; "Documentation for this crate is thin" **C** | 520 commits in 12 months; one minor release every 8–10 months **C** |
| Production embedders | AWS LLRT, Javy (Shopify Functions), SurrealDB **C** | Deno, Supabase edge-runtime **C** | Deno, `rustyscript` **C** | `revm-inspectors` and other tools **C** |

## 3. Measurements

### 3.1 Setup

- **Host:** Apple M3 Max (14 cores), macOS 27.0, Rust 1.93.0. Docker 29.7.2 runs an aarch64 Linux VM for §3.6.
- **Load:** load average 118 when the session started, then 10–27 during the clean-build pass. This is the same loaded workstation as earlier lab reports. Medians are indicative, and p90 values are noisy.
- **Builds:** one Cargo project per engine, all with the same dependencies: `tokio` 1.53.2 (`rt`, `macros`, `time`) and `serde_json`. Release profile with `strip = true` and `lto = "thin"`.
- **Crate versions (exact pins):**
  - `rquickjs = "=0.14.0"`, features `futures`, `loader`. A second build adds `rust-alloc`. The `macro` feature failed to resolve `rquickjs_macro` and was dropped, since it was not needed.
  - `boa_engine = "=0.22.0"` with default features.
  - `v8 = "=152.2.0"`.
  - `deno_core = "=0.412.0"` with default features (V8).
  - `deno_core = "=0.412.0"` with `default-features = false` and `features = ["quickjs", "reactor-tokio"]`.
- **Baseline:** the same Tokio and serde_json program with no engine, 471,408 bytes.
- **Code:** the harness lives in the session scratchpad and is not committed. It can be rebuilt from this description, and the JavaScript inputs are quoted verbatim below.

**JavaScript inputs (identical in every engine):**

```js
// hook and empty function
function hook(req){ const h = {}; for (const [k,v] of Object.entries(req.headers)) h[k.toLowerCase()] = v;
  return { url: req.url.replace('http:', 'https:'), method: req.method, headers: h, keep: req.status === 200 && req.size < 1e6 }; }
function nop(){ return 1; }
// argument (230 bytes of JSON)
{"url":"http://example.com/products?page=3","method":"GET","status":200,"size":48213,
 "headers":{"Accept":"text/html","User-Agent":"Goodfetch/1.0","Accept-Language":"en-US,en;q=0.9"}}
// memory bomb (Boa: bounded to 3,000 pushes so the host survives)
let a=[]; while(true){ a.push(new Array(100000).fill(1)); }
// time-limit probes
while(true){}
let x; for(;;) x = new Array(64).fill(1);      // rusty_v8 issue #2088, script (b)
// async catalog-call stand-in: op(i) is a host async fn that awaits tokio::task::yield_now()
(async () => { let s = 0; for (let i = 0; i < 10000; i++) { s += await op(i); } return s; })()
```

**How each engine was driven:**

- **Per call:** the JSON string is parsed into a JS value by the engine, `hook` is called, and the result is stringified back to a Rust `String`. That means `Ctx::json_parse`/`json_stringify` in `rquickjs`, `v8::json::parse`/`stringify` in `v8`, and `JsValue::from_json`/`to_json` from a `serde_json::Value` in Boa. Each round is 10,000 calls, with three rounds after construction.
- **Time limits:**
  - `rquickjs`: `Runtime::set_interrupt_handler` with a closure returning `Instant::now() > deadline`.
  - `v8`: a watchdog thread calls `IsolateHandle::terminate_execution()` once at the deadline. It exits the process if `run()` has not returned 5 s later.
  - Boa: `RuntimeLimits::set_loop_iteration_limit(10_000_000)`.
- **Memory limits:**
  - `rquickjs`: `Runtime::set_memory_limit(32 MiB)`.
  - `v8`: `CreateParams::heap_limits(0, 32 MiB)` plus `add_near_heap_limit_callback`. The callback calls `terminate_execution()` and returns twice the current limit.
  - Boa: no API exists.

### 3.2 Size and build time (macOS arm64, `aarch64-apple-darwin`)

Command, per crate: `cargo clean && cargo build -q --release`. All crates were built back to back by one script. Stripped size is `stat -f %z`.

| Binary | Stripped size | Added over baseline | Clean build (wall) |
|---|---|---|---|
| baseline (Tokio, serde_json) | 471,408 B | — | 5 s |
| `rquickjs` 0.14.0 | 1,607,760 B | **+1.14 MB** | 13 s |
| `boa_engine` 0.22.0 | 10,779,072 B | **+10.31 MB** | 35 s |
| `v8` 152.2.0 | 43,247,088 B | **+42.78 MB** | 33 s (includes the prebuilt download) |
| `deno_core` 0.412.0 (V8) | 44,965,536 B | **+44.49 MB** | 31 s |
| `deno_core` 0.412.0 (`quickjs`) | 7,268,720 B | **+6.80 MB** | 50 s |

### 3.3 Startup and per-call cost (macOS arm64)

| Engine | Cold runtime + context + compile hook + 1 call (p50 / p90) | New context on existing runtime (p50) | Hook call, JSON in/out (3 rounds) | Empty call | Awaited async op (10k) |
|---|---|---|---|---|---|
| `rquickjs` | 61.6 / 63.8 µs (n=200) | 51.7 µs | 5.50, 5.20, 4.79 µs | 0.019 µs | 0.57 µs |
| Boa | 153.7 / 579.5 µs (n=200) | — | 11.70, 11.81, 11.93 µs | 0.055–0.061 µs | 2.05 µs |
| `v8`, JIT | 475.9 / 635.2 µs (n=100) | 146.6 µs | 1.20, 1.04, 0.96 µs | 0.031–0.037 µs | 0.20 µs (no Tokio) |
| `v8`, `--jitless` | 416.1 / 559.1 µs | 143.5 µs | 1.46, 1.42, 2.26 µs | 0.032–0.054 µs | 0.21 µs (no Tokio) |
| `deno_core` (V8) | 2,788 / 3,088 µs (n=50, `JsRuntime::new` plus the hook script) | — | not run | sync fast op JS→Rust 0.036 µs | 13.6–16.1 µs pending (`yield_now`); 0.08–0.09 µs ready |
| `deno_core` (`quickjs`) | 8,361 / 8,687 µs (n=50) | — | not run | sync op 0.283 µs | 15.9 µs pending; 0.64 µs ready |

Peak RSS of the whole bench process (`/usr/bin/time -l`): `rquickjs` 30 MB, `v8` 69 MB, `deno_core` 25 MB **M**. Boa's RSS reflects the 1.15 GB memory probe, not its baseline.

Reading:

- **All figures are microseconds.** A page fetch costs milliseconds, so the engine choice does not set hook throughput.
- **Where V8 wins:** it is about 5x faster per hook call than QuickJS-NG once warm.
- **Where V8 loses:** each isolate costs 8x more to create than a QuickJS-NG runtime and adds about 40 MB more binary.
- **`deno_core`'s pending-op cost** of about 14 µs is the event-loop round trip per op, measured with an op that yields once. Pending ops in `rquickjs` cost 0.57 µs.

### 3.4 Limit probes (macOS arm64)

| Probe | `rquickjs` | `v8` JIT | `v8` `--jitless` | Boa |
|---|---|---|---|---|
| 32 MiB memory bomb | Error after 11 ms. The exception value is `null` with the system allocator, and `Error: out of memory` with `rust-alloc` and on musl. Context reusable (`1+1=2`) | Near-heap-limit callback at 27.3 MB, terminated after 15 ms, isolate reusable | Same, 12–20 ms | No cap: 3,000 × 100k-element arrays allocated in 30.6 s, 1.15 GB max RSS |
| Same, **without** the near-heap-limit callback | — | **Process abort**, exit 133. "Fatal JavaScript out of memory: Reached heap limit" goes to stderr | — | — |
| JS catches the out-of-memory error in a loop (300 ms interrupt backstop) | Caught **156 times**; the interrupt stopped it at **1.3 s** | — | — | — |
| `while(true){}`, 100 ms deadline | `interrupted` at 100.0 ms, uncatchable, reusable | Terminated at 99.9 ms, reusable | 99.9 ms | Loop limit (10M iterations) hit at 98.5 ms, `RuntimeLimitError`, reusable |
| `let x; for(;;) x = new Array(64).fill(1);` (rusty_v8 #2088), 100 ms and 2 s deadlines | Interrupted at 102 ms | **Not terminated**; still running 5 s later | Terminated at 105 ms | — |
| Same with `V8_FLAGS=--no-turbofan`, 500 ms deadline | — | Terminated at 505 ms | — | — |
| `for(;;) big.fill(1)`, 100 ms deadline | **1.60 s** (100k elements), **16.6 s** (1M elements) | — | — | — |
| `/(a+)+$/.test('a'.repeat(40)+'!')`, 100 ms deadline | Interrupted at 100.1 ms (the regexp engine polls) | — | — | Not run; upstream issue says it hangs **C** |
| Unbounded recursion | — | — | — | `RuntimeLimitError: reached the maximum number of recursive calls` |

### 3.5 macOS hardened runtime and `allow-jit`

Commands:

```sh
codesign -f -s - --options runtime v8b                       # hardened runtime, no entitlements
codesign -f -s - --options runtime --entitlements allow-jit.plist v8b-jit
```

`codesign -dv` shows `flags=0x10002(adhoc,runtime)`.

| Binary | Result **M** |
|---|---|
| `v8b`, JIT, no entitlements | Abort, exit 133: `Fatal process out of memory: Failed to reserve virtual memory for CodeRange` |
| `v8b` with `JITLESS=1` (`--jitless`), no entitlements | All probes pass, same figures as unsigned |
| `v8b-jit` with `com.apple.security.cs.allow-jit` | Runs (exit 3 only from the #2088 watchdog) |
| `denoc` (`deno_core`, default JIT), no entitlements | Same abort as `v8b` JIT |
| `rqjs`, no entitlements | All probes pass |

Caveat: this is an ad hoc signature with the hardened runtime flag, not a Developer ID notarized build. The runtime enforcement of `MAP_JIT` is the same mechanism, but notarization itself was not exercised.

### 3.6 Linux musl (aarch64-unknown-linux-musl, native in a container)

Command:

```sh
docker run --rm -v "$SCRATCH/jsbench:/work" rust:1.93-alpine sh /work/linux_build.sh arm64
```

The script runs `apk add build-base clang-dev linux-headers python3 file`, then `cargo build --release` for each crate with its own target directory, then runs the bench. Rust on `*-linux-musl` links statically by default.

**Environment:** Docker VM with 14 CPUs and 7.7 GiB of memory, Rust 1.93.1, image `rust:1.93-alpine`. Builds ran on a bind-mounted target directory, so wall times are slow and are not comparable with §3.2. `file` reports "statically linked" for every binary below.

| Crate | Build | Stripped size | Run |
|---|---|---|---|
| `rquickjs` | OK, 51 s | 1,839,552 B | All probes pass. Out-of-memory surfaces as `Error: out of memory`. Hook call 4.73–4.78 µs, cold start 83 µs, async op 0.65 µs. The `fill` overshoot repeats: 2.3 s and 22.9 s for a 100 ms deadline |
| `boa_engine` | OK, 544 s | 10,621,360 B | Runs. Hook call 14.1–14.3 µs. The memory probe again ran unbounded (51 s). Async op 10.7 µs |
| `v8` | **Link failed** with default flags: `undefined reference to '__clear_cache'` from `v8/src/codegen/arm64/cpu-arm64.cc`. **OK with `RUSTFLAGS="-C link-arg=-lgcc"`**, 160 s | 46,976,288 B | JIT: limits work, but **#2088 also reproduces here** (watchdog exit 3). Hook call 2.98–5.43 µs, cold isolate 2.0 ms. `--jitless`: every probe passes, hook call 4.9–5.3 µs, cold isolate 2.3 ms |
| `deno_core` (V8) | Same link error; OK with `-lgcc`, 356 s | 48,827,320 B | Runs. Cold `JsRuntime` 13 ms (p50), pending async op 3.8 µs, ready op 0.28 µs |

V8 was 3–5x slower per call and at startup in this container than on macOS, while `rquickjs` was not. The probable causes are musl's allocator (ADR 0024 plans mimalloc) and the VM, but this was not isolated.

### 3.7 x86_64-unknown-linux-musl (emulated container)

Command: `docker run --rm --platform linux/amd64 … rust:1.93-alpine sh /work/linux_x64.sh`. The container runs x86-64 under Docker Desktop's emulation on the arm64 host, so these timings only show that things work.

| Crate | Build | Stripped size | Run (`--jitless` for V8) |
|---|---|---|---|
| `rquickjs` | OK with default flags, 145 s | 2,036,712 B, `static-pie linked` | All probes pass. The `fill` overshoot repeats (5.1 s and 45 s for a 100 ms deadline, slowed by emulation) |
| `v8` | OK with default flags (no `-lgcc` needed), 156 s | 48,521,088 B, `static-pie linked` | All probes pass, including the #2088 loop in about 100 ms |

Not built for x64: Boa (pure Rust, no target-specific code expected) and `deno_core`. V8's JIT mode was not run under emulation.

## 4. Per-candidate notes

### 4.1 QuickJS-NG through `rquickjs`

- **Which QuickJS it binds:**
  - `rquickjs` 0.14.0 vendors QuickJS-NG, whose `quickjs.h` defines `QJS_VERSION` 0.16.2 **M**.
  - The submodule points to quickjs-ng; the tag pins NG commit `0fdea21` (2026-09-13), between NG 0.16.2 and 0.17.0 ([.gitmodules](https://github.com/DelSkayn/rquickjs/blob/master/.gitmodules), [commit](https://github.com/quickjs-ng/quickjs/commit/0fdea21ff1090084e91dad812b343d92e79ba9d9)).
  - The switch from Bellard's QuickJS to NG happened in 0.9.0 ([CHANGELOG](https://github.com/DelSkayn/rquickjs/blob/master/CHANGELOG.md)).
  - QuickJS-NG's latest release is v0.17.0 (2026-09-18) ([releases](https://github.com/quickjs-ng/quickjs/releases)).
- **Features** ([Cargo.toml](https://github.com/DelSkayn/rquickjs/blob/master/Cargo.toml)):
  - `futures`: Promise and Future interop.
  - `parallel`: makes the runtime `Send`/`Sync`; the README calls it experimental.
  - `loader`: custom module resolvers and loaders.
  - `dyn-load`: native modules via `dlopen2`. Code mode should leave it off.
  - `macro`: class and derive macros.
  - `rust-alloc`: uses Rust's global allocator, so mimalloc applies (ADR 0024).
  - `bindgen` is optional, since pre-generated bindings ship for each target.
  - `classes`, `properties`, `array-buffer` and `allocator` are now no-ops.
- **Bindings and build:**
  - Bundled bindings cover `x86_64`/`aarch64-unknown-linux-musl`, both macOS, and `x86_64`/`aarch64-pc-windows-msvc` **M** (`sys/src/bindings/` in the crate).
  - CI tests musl x64/arm64 through zigbuild and qemu, and Windows on `windows-11-arm` ([ci.yml](https://github.com/DelSkayn/rquickjs/blob/master/.github/workflows/ci.yml)).
  - MSRV 1.87 ([crates.io](https://crates.io/api/v1/crates/rquickjs)).
- **Async model:**
  - `Async(fn)` wraps a Rust `async fn` as a JS function returning a Promise (`Promised(fut).into_js`), and `Promise::into_future()` awaits a JS Promise from Rust **M** (from source, `value/function/into_func.rs`, `value/promise.rs`).
  - Futures spawned by JS run on the runtime's own scheduler, driven by `AsyncRuntime::idle()` or `drive()`.
  - Without `parallel`, the runtime is `!Send`, so it must live on a `LocalSet` or a dedicated thread. Closures passed to `async_with` have to be `ParallelSend` ([markers.rs](https://github.com/DelSkayn/rquickjs/blob/master/core/src/markers.rs), [issue #495](https://github.com/DelSkayn/rquickjs/issues/495)).
  - The `async_with!` macro is deprecated in favour of `AsyncContext::async_with(async |ctx| …)` **M** (compiler warning).
  - Open issues: no way to cancel spawned tasks ([#220](https://github.com/DelSkayn/rquickjs/issues/220)), and a segfault with a minimal builder ([#479](https://github.com/DelSkayn/rquickjs/issues/479)). The 0.12.0 changelog fixed GC aborts when an interrupt handler meets async code ([#663](https://github.com/DelSkayn/rquickjs/issues/663)).
- **Writes to stdout:**
  - `AsyncRuntime::idle()` reports failed jobs with `println!("error executing job: …")` at `core/src/runtime/async.rs` lines 325 and 328 **M** (from source; [file](https://github.com/DelSkayn/rquickjs/blob/v0.14.0/core/src/runtime/async.rs)).
  - QuickJS-NG's `js_free_rt` prints and then `abort()`s on an accounting underflow ([quickjs.c](https://raw.githubusercontent.com/quickjs-ng/quickjs/v0.17.0/quickjs.c)).
  - Both conflict with the rule that only the protocol transport writes stdout. Embedding would need `drive()` with its own error handling, or a patch.
- **How the limits work:**
  - The interrupt counter is the compile-time constant `JS_INTERRUPT_COUNTER_INIT 10000`, decremented at 21 poll sites (backward jumps, calls, the regexp engine) **M** (from source).
  - Interrupt throws an **uncatchable** `InternalError: interrupted`. Out-of-memory (`InternalError: out of memory`) and stack overflow (`RangeError`) **are catchable** by JS ([quickjs.c](https://raw.githubusercontent.com/quickjs-ng/quickjs/v0.17.0/quickjs.c)), as §3.4 shows.
  - The `rquickjs` docs say `set_memory_limit` is a no-op with `rust-alloc` ([raw.rs](https://github.com/DelSkayn/rquickjs/blob/v0.14.0/core/src/runtime/raw.rs)). The limit did apply with `rust-alloc` here **M**: NG checks `malloc_limit` before calling the allocator.
  - The default stack limit is 256 KiB ([CHANGELOG](https://github.com/DelSkayn/rquickjs/blob/master/CHANGELOG.md)).
- **ECMAScript coverage:**
  - NG's own `test262_errors.txt` lists 49 failures at v0.17.0 for its enabled feature set ([file](https://github.com/quickjs-ng/quickjs/blob/v0.17.0/test262_errors.txt)). The skipped set includes Intl, Temporal, decorators, ShadowRealm and tail calls ([test262.conf](https://github.com/quickjs-ng/quickjs/blob/v0.17.0/test262.conf)).
  - test262.fyi, run of 2026-10-12: 44,774 of 53,616 overall; `language` 23,288 of 23,726 (V8: 23,077); `intl402` 23 of 3,382 ([data](https://data.test262.fyi/index.json)).
  - Modern features fully passing on test262.fyi: Array.fromAsync, iterator helpers, Set methods, RegExp `v` flag, Float16Array, Error.isError, Math.sumPrecise and Uint8Array base64 ([meta](https://data.test262.fyi/meta.json)).
  - Mostly passing: `using` (480/483), Promise.try (13/15) and RegExp.escape (20/21) (same source).
  - NG says Intl is unlikely ever to be supported ([es_features](https://quickjs-ng.github.io/quickjs/es_features)).
- **Bellard's QuickJS** for comparison ([Changelog](https://github.com/bellard/quickjs/blob/master/Changelog)):
  - Releases 2025-04-26, 2025-09-13 and 2026-06-04. The last adds iterator helpers and Set methods, and claims a 42% speed-up on bench-v8.
  - An unreleased commit of 2026-10-10 adds "experimental AI coded Temporal and Intl support" ([commit](https://github.com/bellard/quickjs/commit/97f64ef96f19a47a23665f4d19c4a689e9a1e42c)).
  - Bellard's QuickJS is on OSS-Fuzz ([project](https://github.com/google/oss-fuzz/tree/master/projects/quickjs)), but has no Rust binding as current as `rquickjs`.
- **Security:**
  - NVD keyword "quickjs" gives 26 relevant CVEs since 2021: 1 in 2021, 1 in 2023, 3 in 2024, 11 in 2025 and 10 so far in 2026, mostly high or medium, including one 2026 critical in the bytecode reader ([NVD](https://services.nvd.nist.gov/rest/json/cves/2.0?keywordSearch=quickjs)). They include use-after-free, type confusion and overflows.
  - QuickJS-NG's 2026 GitHub advisories ([list](https://github.com/quickjs-ng/quickjs/security/advisories)) include remote code execution via `toReversed`, fixed in 0.15.0. Several advisories are memory corruption on out-of-memory paths, fixed in 0.16.0. Those are exactly the paths a memory limit makes reachable.
  - GHSA-52gx-967m-hxp3 named `rquickjs` ≤ 0.12.1 as affected.
  - SurrealDB's CVE-2026-63762 was a DoS through `rquickjs` 0.9.0 (NVD).
  - **On 2026-10-11 NG adopted a policy of "no longer accept private security reports nor issue CVEs"** ([SECURITY.md](https://github.com/quickjs-ng/quickjs/blob/master/SECURITY.md), [PR #1818](https://github.com/quickjs-ng/quickjs/pull/1818), merged 2026-10-11T15:18Z). Fixes will appear only as public issues and releases, so a downstream must follow NG's tracker to learn of them.
  - Untrusted bytecode is out of scope. Code mode must only ever compile source.
  - NG is not on OSS-Fuzz. Its `fuzz.c` targets only the bytecode reader, and CI runs ASan, UBSan, MSan and TSan with test262 ([workflows](https://github.com/quickjs-ng/quickjs/tree/master/.github/workflows)).
  - No RustSec advisory exists for `rquickjs` ([OSV](https://api.osv.dev/v1/query)).
- **Maintenance:**
  - `rquickjs`: 396 commits in the last 12 months, mostly from Richard Davison (LLRT) and Émile Fugulin (Caido) ([repo](https://github.com/DelSkayn/rquickjs)).
  - QuickJS-NG: saghul (129 commits) and bnoordhuis (85) in the last 12 months; about one release every 2 months ([diff page](https://quickjs-ng.github.io/quickjs/diff)).
- **Embedders:**
  - AWS LLRT ([Cargo.toml](https://github.com/awslabs/llrt/blob/main/llrt_core/Cargo.toml)).
  - Javy ([Cargo.toml](https://github.com/bytecodealliance/javy/blob/main/crates/javy/Cargo.toml)), which compiles Shopify Functions ([Shopify](https://shopify.dev/docs/apps/build/functions/programming-languages/javascript-for-functions)).
  - SurrealDB ([Cargo.toml](https://github.com/surrealdb/surrealdb/blob/main/Cargo.toml)).

### 4.2 V8 through the `v8` crate (rusty_v8)

- **Version and cadence:**
  - 152.2.0 (2026-08-20) bundles V8 15.2.124.1 ([README](https://github.com/denoland/rusty_v8/blob/v152.2.0/README.md)); the bench printed `15.2.124.1-rusty` **M**.
  - The README promises a major every 4 weeks, tied to Chrome. Chrome moved to a 2-week cycle with Chrome 153 on 2026-09-08 ([Chrome blog](https://developer.chrome.com/blog/chrome-two-week-release)); stable is 155 ([chromiumdash](https://chromiumdash.appspot.com/fetch_releases?channel=Stable&platform=Linux&num=12)).
  - There has been no crate release since 2026-08-20 ([commits](https://github.com/denoland/rusty_v8/commits/main)).
  - 7 semver-major releases in 12 months ([releases](https://github.com/denoland/rusty_v8/releases)).
  - The scope API changed again around these versions: `v8::scope!` and pinned scopes **M** (examples).
- **Prebuilts** ([release v152.2.0](https://github.com/denoland/rusty_v8/releases/tag/v152.2.0)):
  - Static libraries for all six V1 targets, about 38–41 MB gzipped each. The macOS arm64 archive unpacks to 158 MB **M**.
  - Musl prebuilts arrived in v150.2.0 through [PR #2016](https://github.com/denoland/rusty_v8/pull/2016) (x64, merged 2026-07-15) and [PR #2018](https://github.com/denoland/rusty_v8/pull/2018) (arm64, 2026-07-16). They are cross-compiled and build-only in CI.
  - Pointer-compression prebuilts exist only for macOS and linux-gnu x64.
  - `V8_FROM_SOURCE=1` "often takes 30 minutes". `RUSTY_V8_ARCHIVE`, `RUSTY_V8_ARCHIVE_SHA256` and `RUSTY_V8_MIRROR` let CI pin or mirror the archive (README).
- **`--jitless`:**
  - V8 runs "without any runtime allocation of executable memory" ([v8.dev/blog/jitless](https://v8.dev/blog/jitless), 2019). Its cost was about 40% on Speedometer 2 and about 80% on the Web Tooling Benchmark, and WebAssembly is unavailable.
  - Here the per-call hook cost rose from about 1.0 to about 1.4–2.3 µs **M**.
  - It is runtime-switchable by flag **M**. rusty_v8 also builds iOS with build-time `v8_jitless=true` because iOS denies the JIT entitlement (README).
- **Limits:**
  - When the heap limit is reached and no callback raises it, "V8 will crash with V8::FatalProcessOutOfMemory" ([isolate_create_params.rs](https://github.com/denoland/rusty_v8/blob/v152.2.0/src/isolate_create_params.rs)); confirmed **M**.
  - `IsolateHandle` is `Send`. `terminate_execution` raises an uncatchable termination, and `cancel_terminate_execution` makes the isolate reusable **M** ([isolate.rs](https://github.com/denoland/rusty_v8/blob/v152.2.0/src/isolate.rs)).
  - [Issue #2088](https://github.com/denoland/rusty_v8/issues/2088) (open, 2026-10-06, reported on 150.4.0): termination is ignored by TurboFan-optimized loops calling `Array.prototype.fill`. Reproduced here on 152.2.0, macOS arm64 **M**.
  - ArrayBuffer backing stores are not counted by the heap limit. Deno closed a request to cap them and suggested subprocesses with cgroups ([deno#26202](https://github.com/denoland/deno/issues/26202)).
- **Startup:**
  - The V8 blog reports about 40 ms for a context without a snapshot and under 2 ms with one on desktop (2015) ([custom startup snapshots](https://v8.dev/blog/custom-startup-snapshots)).
  - Current V8 already ships a built-in snapshot: 476 µs per isolate and 147 µs per extra context **M**.
  - Cloudflare cites "a few milliseconds" per isolate for Dynamic Workers ([blog](https://blog.cloudflare.com/dynamic-workers/)).
- **Security:**
  - NVD CVEs whose description says "in V8 in Google Chrome": 55 (2024), 46 (2025) and 137 (2026 to 10-12) ([NVD API](https://services.nvd.nist.gov/rest/json/cves/2.0)).
  - CISA KEV in-the-wild V8 CVEs: 6 (2024), 4 (2025) and 4 (2026) ([KEV feed](https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json)).
  - Two 2026 KEV entries, CVE-2026-85046 and CVE-2026-87491, were fixed in Chrome 152.0.7977.82 and 153.0.8010.36, after the crate's V8 pin of 2026-07-29. Whether 15.2.124.1 is affected was not verified.
  - The V8 Sandbox is "not yet a strong security boundary" ([v8.dev/blog/sandbox](https://v8.dev/blog/sandbox)). In rusty_v8 it is an experimental feature with no prebuilts, so the shipped libraries set `v8_enable_sandbox=false` (README, `build.rs`).
  - V8 is fuzzed by ClusterFuzz ([ClusterFuzz](https://google.github.io/clusterfuzz/)). Its bug volume reflects the most-attacked engine, not the least safe one.
- **Licence:** V8 BSD-3-Clause ([LICENSE](https://github.com/denoland/v8/blob/c4ca1eccb90c5464d826b7713dc27178a53b0cfe/LICENSE)). The static library bundles abseil, ICU, libc++, simdutf, highway and others ([third_party](https://github.com/denoland/rusty_v8/tree/v152.2.0/third_party)), whose notices must ship.

### 4.3 V8 through `deno_core` (and `rustyscript`)

- **Version and churn:**
  - `deno_core` 0.412.0 (2026-09-16) now lives in `denoland/deno/libs/core`; the standalone repo is archived ([repo](https://github.com/denoland/deno_core), [README](https://github.com/denoland/deno/blob/main/libs/core/README.md)).
  - The README says "Documentation for this crate is thin".
  - 47 breaking 0.N releases in 12 months ([versions](https://crates.io/api/v1/crates/deno_core/versions)).
  - It resolves to `v8` 150.4.0, not 152.2.0 **M** (lockfile). The bump PR has been open since 2026-08-22 ([deno#36665](https://github.com/denoland/deno/pull/36665)).
- **Model:**
  - `#[op2] async fn` becomes a Promise. Ops are polled eagerly by default ([op2 README](https://github.com/denoland/deno/blob/main/libs/ops/op2/README.md)).
  - `JsRuntime` is driven by `run_event_loop` on a current-thread Tokio runtime ([op2 example](https://github.com/denoland/deno/blob/main/libs/core/examples/op2.rs)).
  - `add_near_heap_limit_callback` is documented as the way "to prevent V8 from crashing the process" ([jsruntime.rs](https://github.com/denoland/deno/blob/main/libs/core/runtime/jsruntime.rs)).
- **Measured:**
  - A cold `JsRuntime::new` takes 2.8 ms, because the deno_core bootstrap JS runs without a custom snapshot. Embedders normally build one with `JsRuntimeForSnapshot`.
  - A pending async op costs about 14 µs, a ready op 0.09 µs, a fast sync op 0.036 µs.
  - JIT aborts under the hardened runtime without `allow-jit`.
- **Engine facade:**
  - Since 0.4x, `deno_core` depends on **`deno_v8`** 0.4.0, "JavaScript engine facade for Deno", first published 2026-08-04 ([crates.io](https://crates.io/api/v1/crates/deno_v8), [Cargo.toml](https://github.com/denoland/deno/blob/main/libs/deno_v8/Cargo.toml)).
  - Its `quickjs` feature swaps in **`v8x`** 149.4.0-rc.4 (littledivy, MIT OR Apache-2.0). `v8x` implements rusty_v8's C ABI on QuickJS-NG 0.15.1 with 92 local patches ([v8x](https://github.com/littledivy/v8x)), introduced by [deno#36194](https://github.com/denoland/deno/pull/36194) "feat: add experimental QuickJS backend" (merged 2026-08-04).
  - It built and ran here: +6.8 MB, 8.4 ms per cold runtime, 0.28 µs per sync op **M**.
  - `v8x` self-reports passing its rusty_v8 and `deno_core` suites ([status](https://github.com/littledivy/v8x/blob/main/tests/status/report.json)).
  - It has 44 stars, a single author and an rc version.
- **`rustyscript`** 0.12.3 (2025-08-21, MIT OR Apache-2.0):
  - Wraps `deno_core` with `timeout`, `max_heap_size` and a no-filesystem, no-network default ([inner_runtime.rs](https://github.com/rscarson/rustyscript/blob/master/src/inner_runtime.rs)).
  - It is pinned to `deno_core` ^0.355, with no push since 2026-03-24 ([repo](https://github.com/rscarson/rustyscript)).
- **Supabase edge-runtime** pins forked `deno_core` 0.324 and rusty_v8 130 ([Cargo.toml](https://github.com/supabase/edge-runtime/blob/main/Cargo.toml)). Following `deno_core` costs real work even for a company built on it.

### 4.4 Boa (`boa_engine`)

- **Release:** 0.22.0 (2026-08-28). Releases: 0.20 (2024-12), 0.21 (2025-10), 0.21.1 (2026-03) ([releases](https://github.com/boa-dev/boa/releases)). Only the latest 0.x is supported ([SECURITY.md](https://github.com/boa-dev/boa/blob/main/SECURITY.md)).
- **Conformance:** 95.6% test262 for v0.22, and 96.02% on `main` ([v0.22](https://raw.githubusercontent.com/boa-dev/data/main/test262/refs/tags/v0.22/latest.json), [main](https://raw.githubusercontent.com/boa-dev/data/main/test262/refs/heads/main/latest.json)). It includes Temporal, on by default ([v0.22 notes](https://github.com/boa-dev/boa/releases/tag/v0.22)).
- **Performance:** 216 on Boa's own v8-v7 score, against 1,237 for QuickJS and 2,515 for V8 jitless ([score.json](https://raw.githubusercontent.com/boa-dev/data/main/bench/results/score.json)). The per-call cost here was about 2.3x QuickJS-NG's **M**.
- **Limits:**
  - `RuntimeLimits` sets a loop-iteration limit (default unlimited, counted per call frame), recursion (512), stack (10,240) and backtrace (50) ([runtime_limits.rs](https://raw.githubusercontent.com/boa-dev/boa/v0.22/core/engine/src/vm/runtime_limits.rs), [loop_ops.rs](https://github.com/boa-dev/boa/blob/main/core/engine/src/vm/opcode/iteration/loop_ops.rs)).
  - Limit errors cannot be caught by JS.
  - **No interrupt from another thread:** [#3442](https://github.com/boa-dev/boa/issues/3442) has been open since 2023-10-30, and the `Arc<AtomicBool>` interrupt PR [#5475](https://github.com/boa-dev/boa/pull/5475) was closed unmerged on 2026-08-10.
  - The instruction budget exists only under `cfg(feature = "fuzz")` ([error/mod.rs](https://github.com/boa-dev/boa/blob/main/core/engine/src/error/mod.rs)).
  - `Script::evaluate_async_with_budget` yields to the executor, but builtins written in Rust ignore the budget (#3442).
  - RegExp has no backtracking limit ([#4643](https://github.com/boa-dev/boa/issues/4643)).
  - **No heap cap:** the GC heap is a `thread_local!` shared by every `Context` on the thread, with a growing collection threshold **M** (from source, `boa_gc/src/lib.rs` lines 44–68).
- **Async:**
  - `NativeFunction::from_async_fn` (the closure must be `Copy`) and `JobExecutor::run_jobs_async` **M**.
  - The default executor (`IdleJobExecutor`) disables promises ([job docs](https://docs.rs/boa_engine/0.22.0/boa_engine/job/index.html)).
  - `Context` is `!Send` ([docs](https://docs.rs/boa_engine/0.22.0/boa_engine/context/struct.Context.html)).
- **Safety:**
  - Written in Rust, but the engine, GC and string crates use `unsafe` hundreds of times (rough grep count).
  - 1 advisory: [GHSA-f67q-wr6w-23jq](https://github.com/boa-dev/boa/security/advisories/GHSA-f67q-wr6w-23jq), high, 2024.
  - `cargo-fuzz` targets exist in the repo; Boa is not on OSS-Fuzz.
- **Activity:** 520 commits in 12 months, about 103 of them from dependabot ([commits API](https://api.github.com/repos/boa-dev/boa/commits?since=2025-10-12T00:00:00Z)).

### 4.5 Other options (brief)

- **QuickJS or SpiderMonkey compiled to WebAssembly, run in wasmtime:**
  - **Isolation:** the strongest of all options: the guest cannot touch host memory, and the host has fuel or epoch interruption plus `StoreLimits` ([interrupting](https://docs.wasmtime.dev/examples-interrupting-wasm.html), [StoreLimitsBuilder](https://docs.rs/wasmtime/latest/wasmtime/struct.StoreLimitsBuilder.html)).
  - **Guest engines:**
    - Javy CLI 9.1.0 is built on `rquickjs` 0.14 ([repo](https://github.com/bytecodealliance/javy)).
    - StarlingMonkey 0.3.0 is SpiderMonkey/Firefox 147. Riza uses it for an LLM code-interpreter API ([ADOPTERS](https://github.com/bytecodealliance/StarlingMonkey/blob/main/ADOPTERS.md)).
    - ComponentizeJS is at 0.23.0.
  - **macOS signing:** Cranelift code, even precompiled `.cwasm`, is made executable at run time and needs `allow-jit`. `MAP_JIT` support is still open ([wasmtime#11989](https://github.com/bytecodealliance/wasmtime/issues/11989)).
  - **Pulley interpreter:** avoids `allow-jit` but is about 10x slower than Cranelift ([Pulley](https://docs.wasmtime.dev/examples-pulley.html)), and that comes on top of an interpreter running inside it.
  - **Platform support:** Linux musl and Windows arm64 are Tier 3 ([tiers](https://docs.wasmtime.dev/stability-tiers.html)).
  - **Security history:** wasmtime itself has 60 GitHub advisories, 38 in the last 12 months, including a 2026 aarch64 Cranelift miscompile that allowed sandbox escape ([advisories](https://github.com/bytecodealliance/wasmtime/security/advisories)).
  - **Cost:** two engines, plus a host↔guest serialization boundary for every catalog call. Not measured here.
- **Nova (`nova_vm`)** 1.0.0 (2026-03-15) is MPL-2.0, outside ADR 0018's MIT/Apache family ([crates.io](https://crates.io/api/v1/crates/nova_vm)). Its own test262 metric is 79.9% ([metrics](https://github.com/trynova/nova/blob/main/tests/metrics.json)).
- **Hermes / Static Hermes** is MIT, but its last tag is v0.13.0 (2024-08) and it scores 41.8% on test262.fyi. It lacks local `eval`, `with` and ES module loading ([Features.md](https://github.com/facebook/hermes/blob/static_h/doc/Features.md)). The only Rust binding is a 0.0.0 placeholder ([rusty_hermes](https://github.com/rust-hermes/rusty_hermes)).
- **JavaScriptCore:** `javascriptcore-rs` 2.0.0 links the system `javascriptcoregtk` dynamically, so it cannot be used for static musl or Windows ([sys Cargo.toml](https://github.com/tauri-apps/javascriptcore-rs/blob/crate/sys/Cargo.toml)). `rusty_jsc` has been idle since 2023 ([repo](https://github.com/wasmerio/rusty_jsc)).
- **MicroQuickJS** (Bellard, 2025-12) supports only a subset close to ES5. That is too small for model-written modern JS ([repo](https://github.com/bellard/mquickjs)).
- **`mozjs`** (SpiderMonkey) 0.26.8 is MPL-2.0 ([crates.io](https://crates.io/api/v1/crates/mozjs)).
- **Pydantic Monty** is a Python interpreter in Rust for model-written code, not JavaScript ([repo](https://github.com/pydantic/monty)). Its comparison page is a useful survey of sandbox start costs ([alternatives](https://pydantic.dev/docs/monty/reference/alternatives/)).

## 5. What others do

- **Cloudflare Code Mode** runs each model-written snippet in a fresh V8 isolate through the Worker Loader API. `globalOutbound: null` removes the network, and tools are RPC bindings, so keys stay outside the sandbox ([blog](https://blog.cloudflare.com/code-mode/)).
  - Dynamic Workers (2026-03-24) add a second layer and hardware memory protection keys, conceding that V8 bugs are more frequent than hypervisor bugs ([blog](https://blog.cloudflare.com/dynamic-workers/)).
  - Check Point reported a workerd memory-safety bug that it says could escape the Code Mode sandbox ([report](https://research.checkpoint.com/2026/when-agentic-glue-melts/)).
  - Cloudflare patches V8 "within hours". A crate pinned to a monthly `v8` cannot.
- **Deno** treats "the V8 VM [as] a secure sandbox" ([SECURITY.md](https://github.com/denoland/deno/blob/main/.github/SECURITY.md)). It signs its notarized binary with `allow-jit`, `allow-unsigned-executable-memory` and `disable-executable-page-protection` ([entitlements.plist](https://github.com/denoland/deno/blob/main/cli/entitlements.plist)).
- **AWS LLRT** chose QuickJS-NG through `rquickjs` for fast cold starts ([Cargo.toml](https://github.com/awslabs/llrt/blob/main/llrt_core/Cargo.toml)).
- **Shopify Functions** run QuickJS compiled to Wasm by Javy, with the Wasm runtime as the boundary ([Shopify](https://shopify.dev/docs/apps/build/functions/programming-languages/javascript-for-functions)).
- **Riza** runs StarlingMonkey in Wasm for LLM code execution ([ADOPTERS](https://github.com/bytecodealliance/StarlingMonkey/blob/main/ADOPTERS.md)).
- **SurrealDB** embeds `rquickjs` with `rust-alloc` for user functions inside the database ([Cargo.toml](https://github.com/surrealdb/surrealdb/blob/main/Cargo.toml)).
- **Supabase edge-runtime** uses V8 isolates with memory and CPU limits, on a forked `deno_core` ([Cargo.toml](https://github.com/supabase/edge-runtime/blob/main/Cargo.toml)).

## 6. Implications for the decision (inputs, not a decision)

- **Boa is ruled out by the limits contract,** not by speed. It has no heap cap, no external interrupt, and a ReDoS hang. ADR 0011 requires time and memory limits.
- **The in-process limits of QuickJS-NG and V8 both have measured holes,** so the confinement question in #125 does not go away whichever engine is chosen:
  - **QuickJS-NG** has coarse interrupt granularity and a catchable out-of-memory error.
  - **V8 with JIT** has #2088 and aborts the process on an unhandled out-of-memory.
  - **Options:**
    - Run the sandbox in a confined helper process with a hard kill, like the Plugins of ADR 0033.
    - Keep it in process with a dedicated thread and a last-resort process abort.
    - Patch the engine: lower NG's interrupt counter or add poll points in builtins; use `--jitless` for V8.
- **The `--jitless` V8 path removes two problems:** `allow-jit` (ADR 0039) and #2088. In exchange it costs throughput that hooks barely notice (1.4 vs 1.0 µs per call) and 42 MB of binary.
- **`rquickjs` is the smallest and simplest in-process fit.** Its costs are a C engine with a steady memory-safety bug stream, a maintainer that has just stopped issuing CVEs, a stdout write to patch around, and an async model that is `!Send` unless the experimental `parallel` feature is on.
- **`deno_core` adds the most churn** (47 breaking releases a year, an older pinned `v8`) and a 2.8 ms runtime without a custom snapshot, and brings no limit the bare `v8` crate lacks. Its new QuickJS facade is worth watching but is experimental.
- **ECMAScript surface:** all three main candidates cover modern syntax, `async`/`await`, modules and the 2024–2025 built-ins a model is likely to write. Intl is the gap in QuickJS-NG; a sandbox that only calls the catalog may not need it.

## 7. Limitations and what was not verified

- **Not measured:**
  - **Native x86_64-unknown-linux-musl:** it was only built and run under emulation, without timings or V8 JIT (§3.7).
  - **Other targets:** Windows x64 and arm64, and macOS x64. Their status rests on the cited prebuilt lists and CI.
  - Notarized Developer ID signing; only an ad hoc hardened-runtime signature was tested.
  - V8 with a custom startup snapshot, and `deno_core` with `JsRuntimeForSnapshot`.
  - `deno_core` hook calls (the same engine as `v8`).
  - The wasmtime/Javy path. Boa under the hardened runtime: it is an interpreter, so it is expected to run.
- **Timings come from a loaded workstation**, a single run per configuration (three rounds for per-call cost). They are not reference-host figures.
- **Async-op figures are not like for like:**
  - `rquickjs`, Boa and `deno_core` awaited a Tokio `yield_now` inside the op.
  - The `v8` figure uses a hand-rolled resolver loop with no Tokio.
- **Inputs differ slightly:** Boa receives the hook argument from `serde_json::Value` (`JsValue::from_json`) rather than from an engine-side `JSON.parse`.
- **Cited facts not re-verified here:**
  - Whether `v8` 152.2.0 is affected by the two 2026 KEV V8 CVEs fixed after its pin.
  - Whether the `v8` musl prebuilts pass rusty_v8's tests (CI only builds them). §3.6 only shows whether they link and run this bench.
  - Why test262.fyi counts some older QuickJS-NG features lower than NG's own 49-failure list suggests.
  - The exact licences of each V8 third-party component.
  - Wasmtime binary size with Cranelift inside a Rust static binary.
- **Counts:** NVD and GitHub advisory counts are keyword or listing counts on 2026-10-12 and can include duplicates or near-misses.
  - The 2026 V8 count (137, against 46 for the whole of 2025) comes from one keyword query and could not be re-run here, because the NVD API refused the repeat queries.
