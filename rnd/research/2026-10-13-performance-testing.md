# Performance regression testing in CI

Research of **October 13, 2026** for ticket [#272](https://github.com/Jbmanllr/automan/issues/272) (map [#262](https://github.com/Jbmanllr/automan/issues/262)). Question: beyond ADR 0025's release benchmark on the reference host and ADR 0057's Size budget, how do projects catch performance and resource regressions on every change without noisy or flaky results, in Rust code, in protocol and SDK overhead, and in the Console?

**Sources.** Official documentation, source code and workflow files of the tools and projects named, read on 2026-10-13 through `gh api`, the registries' APIs (crates.io, npm) and the documentation sites. Commit heads read: uv `59e5912ecf`, Ruff `350c634ad1`, Biome `eca082b594`, Oxc `027f677d32`, Tokio `1989b46f5a`, Polars `349d619e54`, pydantic `22c4317a5d`. Three published measurements of noise on shared machines, and users' issues for the pains. Everything here is cited; nothing was measured for this note. Claims that rest on one vendor's own measurement are marked as such, and anything not checked against its source is in the Unverified list. Like everything in `rnd/`, this note informs a decision and makes none.

---

## Summary

- **Wall time on shared CI runners is too noisy to gate small regressions.**
  - CodSpeed ran the benchmark suites of uv, Ruff, Turbopack and Reflex 100 times on GitHub-hosted runners and measured a 2.66% coefficient of variation. At that noise, a 2% gate raises a false alarm about 45% of the time, and a 1% false-alarm rate needs a 7% gate (vendor's own measurement, July 2025).
  - GitHub hands out different CPUs under the same runner label (AMD EPYC 7763 in 9 of 10 runs, Intel Xeon 8370C in 1, on `ubuntu-24.04`), and glibc picks different code paths on each.
  - Criterion.rs's own FAQ says not to rely on cloud CI results. A peer-reviewed study found a coefficient of variation from 0.03% to over 100% across benchmarks and cloud instance types.
- **What projects actually gate on each change is deterministic counts.**
  - Instruction counts under Valgrind: CodSpeed's CPU simulation, gungraun (formerly iai-callgrind), rustc-perf, Rustls and SQLite.
  - Exact allocation counts: Oxc's required "Allocations" check fails when a committed snapshot changes.
  - Sizes: Oxc's required "Minification Size" check, size-limit, ADR 0057.
  - Even instruction counts move: by 0.1–0.5% between CI jobs on different CPUs, and by more when code layout, allocator state or hash seeds change.
- **Wall time on every change needs dedicated bare metal.**
  - CodSpeed Macro Runners measured a 0.56% coefficient of variation (vendor's figure), and Bencher offers bare-metal runners.
  - Rustls runs its own bare-metal server with turbo, SMT, ASLR and the NMI watchdog off (per Bencher's case study).
  - CodSpeed's macro runners need a GitHub organization, and public repositories must be allowed in the runner group.
- **CodSpeed is the common choice among the comparable projects, but nobody visibly blocks merges on it.**
  - uv, Ruff, Biome, Oxc and pydantic run CodSpeed.
  - Neither uv's nor Ruff's required-checks job includes benchmarks, and Oxc's required checks do not include CodSpeed's.
  - Polars removed CodSpeed in May 2026 ("we haven't used it in over a year"). A Biome maintainer calls its results "quite unreliable", and Oxc filed issues about noise from reallocations and hash-map resizing.
  - Tokio only compiles its benchmarks.
- **Overhead across a process boundary is outside CPU simulation.**
  - CodSpeed's simulation does not follow child processes, excludes system calls, and its `exec` mode does not support statically linked executables.
  - So an SDK call to an Attached engine is measured either by wall time on bare metal or by deterministic counts. Deno tracks system-call and thread counts with `strace`, and times its JSON-RPC language server over stdio. gRPC measures "contentionless latency" on dedicated machines every few hours, not on each pull request.
- **Console.**
  - Bytes are easy to gate: size-limit 14.2.0 fails CI over a limit.
  - Time is not. size-limit warns that its time plugin "might be unstable". Lighthouse says free CI machines are "typically quite volatile". Lighthouse CI 0.15.1 still bundles Lighthouse 12.6.1, while Lighthouse is at 13.5.0.
  - Reassure, the React render-regression tool, is Jest-only, while Goodfetch's TypeScript tests run on Vitest.
- **Gating methods.**
  - Most tools gate on a fixed percentage against the base branch: CodSpeed defaults to 10%, and github-action-benchmark to 200%.
  - Bencher tests each result against history (t-test, z-score, IQR and others), and rustc-perf flags results beyond Q3 + 3×IQR of its past changes.
  - Change-point detection (MongoDB, Apache Otava) works after merge, on a series. Interleaving the base and the change on the same machine detects slowdowns of 10% or less even in public clouds.

---

## 1. Microbenchmarks in CI

### 1.1 The tools

| Tool | Version (date) | What it measures | Comparison and gate | Platforms | Licence and cost |
|---|---|---|---|---|---|
| criterion | 0.8.2 (2026-02-04) | Wall time, with bootstrap confidence intervals | Saved baselines. Defaults: noise threshold 1%, significance 0.05, confidence 0.95, 100 samples, 5 s measurement, 3 s warm-up. No CI gate built in. | All | Permissive (Apache-2.0/MIT); free |
| divan | 0.1.21 (2025-04-10) | Wall time; allocation counts through `AllocProfiler`; thread contention | No comparison across runs (issue #71 open since 2025-02) | All | Permissive; free |
| gungraun (formerly iai-callgrind) | 0.20.0 (2026-09-26) | Instructions, estimated cycles and cache events (Callgrind, Cachegrind); heap metrics (DHAT); hardware counters (Linux perf) | Soft limits in % against a baseline, hard limits as absolute numbers; exit code 3 on a regression | Where Valgrind runs (Linux, FreeBSD) or Linux perf; not Windows | Apache-2.0; free |
| CodSpeed | action 5.4.2 (2026-10-09), Rust crates 5.0.2 (2026-09-17) | CPU simulation (Valgrind-based cycle estimate), wall time, memory | GitHub status check against the base branch; 10% default threshold | Simulation and memory on Linux | Hosted service, runner Apache-2.0; free for open source |
| Bencher | 0.6.13 (2026-09-28) | Ingests any harness's results | Statistical tests against history; `--error-on-alert` fails the job | Any | Core Apache-2.0/MIT plus "Bencher Plus"; self-hostable; free for public projects |
| tango | 0.8.0 (crates.io, 2026-08-21) | Paired A/B wall time of two builds in one process | Detects "a 1% change in performance within just 1 second in at least 9 out of 10 test runs" (its claim) | macOS, Linux, Windows | Free |

Sources and details:

- **criterion.**
  - The FAQ answers "How Should I Run Criterion.rs Benchmarks In A CI Pipeline?" with "You probably shouldn't (or, if you do, don't rely on the results)". It blames the virtualization of cloud CI and points to Iai, which counts instructions inside Cachegrind ([faq.md](https://github.com/criterion-rs/criterion.rs/blob/master/book/src/faq.md)).
  - The defaults are in [`src/lib.rs`](https://github.com/criterion-rs/criterion.rs/blob/master/src/lib.rs). The analysis treats changes within the noise threshold (for example ±1%) as noise ([analysis.md](https://github.com/criterion-rs/criterion.rs/blob/master/book/src/analysis.md)).
  - 0.8.0 (2025-11-29) added "alloca-based memory layout randomisation to mitigate memory effects on measurements" ([CHANGELOG](https://github.com/criterion-rs/criterion.rs/blob/master/CHANGELOG.md)). The project moved from `bheisler` to the `criterion-rs` organization.
- **divan.**
  - `AllocProfiler` "measures `GlobalAlloc` memory usage" and wraps any allocator, mimalloc included ([docs.rs](https://docs.rs/divan/0.1.21/divan/struct.AllocProfiler.html)).
  - Andrew Gallant (BurntSushi, author of `critcmp`) asked in [divan#71](https://github.com/nvzqz/divan/issues/71) for a workflow to compare runs, saying he "would really love to not build a third tool". It is still open.
  - No release since 0.1.21 (crates.io).
- **gungraun.**
  - Renamed from iai-callgrind at 0.17.0 (2025-09-21); the old crate stops at 0.16.1 ([migration](https://github.com/gungraun/gungraun/blob/main/docs/src/migration/iai-callgrind-to-gungraun.md), [CHANGELOG](https://github.com/gungraun/gungraun/blob/main/CHANGELOG.md)).
  - It "can take accurate measurements with Valgrind even in virtualized CI environments". It "cannot help you much" with wall-clock times, and "cannot be run on Windows" ([README](https://github.com/gungraun/gungraun)).
  - Regression checks are opt-in, for example `--callgrind-limits='ir=5%|10000'` (soft and hard limits), and also exist for DHAT and perf metrics ([regressions.md](https://github.com/gungraun/gungraun/blob/main/docs/src/regressions.md)).
  - A `setup-gungraun` GitHub Action installs the runner and Valgrind ([ci.md](https://github.com/gungraun/gungraun/blob/main/docs/src/installation/ci.md)), and debug symbols are required ([prerequisites](https://github.com/gungraun/gungraun/blob/main/docs/src/installation/prerequisites.md)).
  - Valgrind 3.27.1 (2026-05-20) supports AMD64/Linux "up to and including AVX2" and ARM64/Linux. On Darwin it stops at AMD64 macOS 11.0, so there is no Apple-silicon Mac ([platforms](https://valgrind.org/info/platforms.html), [home](https://valgrind.org/)).
- **CodSpeed.**
  - *CPU simulation.* A benchmark "will be run only once and the CPU behavior will be simulated". It estimates cycles from weighted instructions plus L1 and last-level cache misses, and "does not include system calls in the measurement" ([CPU simulation](https://codspeed.io/docs/instruments/cpu)). The engine is "a slightly modified version" of Callgrind ([blog](https://codspeed.io/blog/why-glibc-faster-github-actions)).
  - *Wall time.* "Measures the actual elapsed time", system calls and I/O included, and is meant for CodSpeed's bare-metal Macro Runners ([walltime](https://codspeed.io/docs/instruments/walltime)).
  - *Memory.* Uses eBPF and runs the benchmark once (§2).
  - *Gate.* The status check fails when a benchmark regresses beyond its threshold. The global default is **10%**, settable from 0% to 50%, and can be overridden per benchmark ([customization](https://codspeed.io/docs/features/customization), [metrics](https://codspeed.io/docs/features/understanding-the-metrics)). In an organization, "only admins are allowed to acknowledge regressions" so that the check passes ([performance checks](https://codspeed.io/docs/features/performance-checks/index)).
  - *Baseline.* There is a baseline only if the default branch also runs CodSpeed on `push` ([metrics](https://codspeed.io/docs/features/understanding-the-metrics)).
  - *Forks.* Tokenless uploads let pull requests from public forks report ([GitHub Actions](https://codspeed.io/docs/integrations/ci/github-actions/index)).
  - *Pricing.* The free plan has unlimited users for open source and 600 macro-runner minutes a month, then $0.032 a minute on Graviton. The Ryzen 9950X runner is not on the free plan ($0.06 a minute on Pro) ([pricing](https://codspeed.io/pricing), [macro runners](https://codspeed.io/docs/features/macro-runners)).
- **Bencher.**
  - A Threshold applies one Test to a branch, testbed and measure. The Tests are percentage, z-score, t-test, log normal, IQR, delta IQR and static ([thresholds](https://bencher.dev/docs/explanation/thresholds/)).
  - `bencher noise` scores a machine's compute, cache and I/O jitter and its CPU steal ([reference](https://bencher.dev/docs/reference/bencher-noise/)).
  - Bare-metal runners execute "only one Job ... at a time", in Firecracker microVMs by default ([bare metal](https://bencher.dev/docs/explanation/bare-metal/)).
  - The free plan for public projects includes on-demand bare-metal runners with 1 concurrent job and a 5-minute job timeout ([pricing](https://bencher.dev/pricing/)).
  - Pull requests from forks get no `BENCHER_API_KEY` secret, so they need a separate workflow ([GitHub Actions how-to](https://bencher.dev/docs/how-to/github-actions/)).
  - Licence: "plus" directories are under the Bencher Plus License, the rest Apache-2.0 or MIT ([LICENSE.md](https://github.com/bencherdev/bencher/blob/main/LICENSE.md)).
- **tango.** Paired benchmarking measures "the performance difference between two simultaneously executing functions" ([README](https://github.com/bazhenov/tango)).

### 1.2 Who uses which

| Project | Harness | What runs in CI | Gating |
|---|---|---|---|
| uv | `codspeed-criterion-compat` 5.0.1 | CodSpeed simulation on Depot `ubuntu-24.04-4`, and wall time on `codspeed-macro-x64-ryzen-9950x-ubuntu-24-04` ([bench.yml](https://github.com/astral-sh/uv/blob/main/.github/workflows/bench.yml)) | `bench` is not in the "all required jobs passed" job, the only check main's ruleset requires ([ci.yml](https://github.com/astral-sh/uv/blob/main/.github/workflows/ci.yml)) |
| Ruff and ty | `codspeed-criterion-compat` 5.0 and `codspeed-divan-compat` 5.0 | CodSpeed `simulation,memory` on Depot runners. Wall time on `codspeed-macro` (the Graviton runner), with flame graphs off because they add about 6 minutes. A separate memory-report workflow diffs ty's own memory report between the change and its merge base ([ci.yaml](https://github.com/astral-sh/ruff/blob/main/.github/workflows/ci.yaml), [memory_report.yaml](https://github.com/astral-sh/ruff/blob/main/.github/workflows/memory_report.yaml)) | Benchmarks are not in `required-checks-passed` |
| Biome | `codspeed-criterion-compat` =5.0.1 | CodSpeed simulation on Depot arm64 runners, filtered by changed paths ([benchmark.yml](https://github.com/biomejs/biome/blob/main/.github/workflows/benchmark.yml)) | Not visible. [biome#6934](https://github.com/biomejs/biome/issues/6934) (open since 2025-07) calls the results "generally slow to run" and "quite unreliable". A maintainer keeps CodSpeed for its history, flame graphs and support for fork pull requests. |
| Oxc | `criterion2` 3.0.4 with a `codspeed` feature | CodSpeed simulation and memory, sharded per component on GitHub-hosted `ubuntu-latest` ([benchmark.yml](https://github.com/oxc-project/oxc/blob/main/.github/workflows/benchmark.yml)) | Main's ruleset requires "Allocations" and "Minification Size" but no CodSpeed check (§2) |
| Tokio | `criterion` 0.5.1 | `cargo check --benches` only; Valgrind leak checks on stress tests ([ci.yml](https://github.com/tokio-rs/tokio/blob/master/.github/workflows/ci.yml), [stress-test.yml](https://github.com/tokio-rs/tokio/blob/master/.github/workflows/stress-test.yml)) | None for performance |
| Polars | pytest | CodSpeed was set up in 2024-04 and removed on 2026-05-21 ([#27686](https://github.com/pola-rs/polars/pull/27686): "we haven't used it in over a year"). Now each pull request gets a comment with the wheel size. TPC-H at scale factor 10 runs on a self-hosted runner for main and for pull requests labelled `needs-bench` ([benchmark.yml](https://github.com/pola-rs/polars/blob/main/.github/workflows/benchmark.yml), [benchmark-remote.yml](https://github.com/pola-rs/polars/blob/main/.github/workflows/benchmark-remote.yml)) | None |
| pydantic (pydantic-core merged in; old repository archived) | `pytest-codspeed` | CodSpeed simulation on GitHub-hosted `ubuntu-24.04`, with pydantic-core built with PGO ([codspeed.yml](https://github.com/pydantic/pydantic/blob/main/.github/workflows/codspeed.yml)) | The ruleset requires `check`; whether the CodSpeed check is also required is not visible |

### 1.3 Noise measured on shared runners

| Source | Environment | Finding |
|---|---|---|
| CodSpeed, [Benchmarks in CI: Escaping the Cloud Chaos](https://codspeed.io/blog/benchmarks-in-ci-without-noise), 2025-07-30 (vendor) | GitHub-hosted x86-64, AMD EPYC 7763, 4 vCPU; 100 runs each of the Turbopack, uv, Ruff and Reflex suites | Coefficient of variation 2.66%. Under a normal model, a 2% gate gives a 45% chance of a false positive, and a 1% false-positive rate needs a 7% gate. On their Macro Runners (Graviton, bare metal): 0.56%, so a 1.5% gate stays under 1% false positives. |
| CodSpeed, [Why glibc is faster on some GitHub Actions Runners](https://codspeed.io/blog/why-glibc-faster-github-actions), 2026-03-03 (vendor) | GitHub-hosted `ubuntu-24.04`, CPU simulation | Rerunning one binary on one machine gives identical results. Rebuilt across 10 jobs, some benchmarks varied by 0.1–0.505% relative standard deviation. The runners were Xeon 8370C (1 of 10) or EPYC 7763 (9 of 10), and glibc's `malloc` takes CPU-dependent paths. The authors "experimentally confirm[ed]" getting the same machine across 100 runs on 8-vCPU larger runners. |
| B. Heisler (author of Criterion.rs), [Are Benchmarks From Cloud CI Services Reliable?](https://bheisler.github.io/post/benchmarking-in-the-cloud/), 2018-01-06 | Travis-CI against a local desktop, regex crate, 100 pairs | Standard deviation of paired differences: 2.11 points locally against 72.15 in the cloud. "Differences of as much as 50% are fairly common with no change in the code at all." |
| Laaber, Scheuner, Leitner, [Software microbenchmarking in the cloud. How bad is it really?](https://doi.org/10.1007/s10664-019-09681-1), Empirical Software Engineering, 2019 ([preprint](https://www.zora.uzh.ch/server/api/core/bitstreams/da610977-cce1-464d-8733-0f899babb3d1/content)) | AWS, GCE and Azure (3 instance types each) and IBM bare metal; Java and Go; over 4.5 million data points | Coefficient of variation from 0.03% to more than 100% depending on benchmark and instance. Running "test and control experiments on the same instances (in randomized order)" detects slowdowns of 10% or less with high confidence. |
| Bulej et al., [Duet Benchmarking](https://arxiv.org/abs/2001.05811), 2020 | Shared cloud machines | Running the two versions in parallel and comparing them together improves accuracy 2.3–12.5× (ScalaBench, DaCapo) and 23.8–82.4× (SPEC CPU 2017). |
| Reassure, [README](https://github.com/callstack/reassure) | Any CI agent | `reassure check-stability` runs the same code twice. Random changes "should be below 5%", and 10% or more is "too high". |

User pains with instruction-based measurement:
- [oxc#4270](https://github.com/oxc-project/oxc/issues/4270) (2024-05): `oxc_semantic` benchmarks are "very unstable" because of `Vec` reallocations, and CodSpeed bombards pull requests with "semantic got 3% faster / 5% slower" messages.
- [oxc#5267](https://github.com/oxc-project/oxc/issues/5267): variance from a `hashbrown::HashSet` resizing unpredictably.
- [gungraun#377](https://github.com/gungraun/gungraun/issues/377): two identical benchmark bodies give different counts, which the maintainer attributes to the compiler and glibc version.
- CodSpeed documents the same causes: toolchain or runner-image updates, inlining and code layout, allocator state, random hash seeds and directory order. Its remedies are to pin the toolchain, give each benchmark its own binary, tune the allocator or exclude allocations, and use deterministic hashers ([variance](https://codspeed.io/docs/instruments/cpu/regression-causes), [reducing variance](https://codspeed.io/docs/instruments/cpu/reducing-variance)).

### Fit for Goodfetch

- **Gating tiers.** ADR 0032 runs pull requests on GitHub-hosted runners, the full matrix in the merge queue, and the reference host only on tags and manual dispatch. By the figures above, a per-change gate on GitHub-hosted runners can only hold deterministic counts (instructions, allocations, sizes) or large thresholds. Wall time per change needs bare metal, through CodSpeed's or Bencher's runners or one Goodfetch operates.
  - The reference host cannot serve that role as ADR 0032 stands, because it never runs pull-request code.
  - CodSpeed's macro runners need the GitHub organization ADR 0032 already plans, plus allowing public repositories on the default runner group.
- **Instruction counts are Linux-only.** Valgrind has no Apple-silicon macOS and no Windows, so CodSpeed simulation and gungraun run only in the Linux jobs. That fits a gate that runs on one platform.
- **STYLE.md conflicts.**
  - STYLE.md allows Cargo features "only on the embeddable `goodfetch` crate". CodSpeed's "one benchmark, one binary" remedy uses a feature per benchmark; separate `[[bench]]` targets would give separate binaries without features.
  - STYLE.md pins the toolchain in `rust-toolchain.toml`, which already removes CodSpeed's first named cause of drift.
- **The engine allocates through mimalloc (ADR 0024).** CodSpeed's memory instrument supports mimalloc, and divan's `AllocProfiler` can wrap it.

---

## 2. Memory and allocations

### 2.1 Heap tests in Rust

- **dhat-rs 0.3.3** (2024-02-04; last push 2025-02-20).
  - "Heap usage testing" puts a `Profiler` in testing mode, reads `HeapStats::get()` (total and peak blocks and bytes, current blocks and bytes), and asserts with `dhat::assert_eq!`.
  - The crate's own warning: it "is experimental", "may crash, hang, or otherwise do the wrong thing", and "its maintenance is not a high priority of the author".
  - Only one `Profiler` may run at a time, and the test runner's own threads allocate. So "the best approach is to put each heap usage test in its own integration test file", or to run with `--test-threads=1`.
  - Heap profiling replaces the global allocator with `dhat::Alloc`, which wraps the system allocator, and should only run in release builds ([lib.rs docs](https://github.com/nnethercote/dhat-rs/blob/main/src/lib.rs)).
- **Oxc's allocation gate.**
  - A tracking global allocator counts system allocations, reallocations and deallocations, bytes, peak growth and arena activity, per file and per stage.
  - `cargo allocs` writes YAML snapshots. The required "Allocations" job runs it and fails on `git diff --exit-code` with "Allocations have changed ... otherwise please fix the regression" ([ci.yml](https://github.com/oxc-project/oxc/blob/main/.github/workflows/ci.yml), [README](https://github.com/oxc-project/oxc/blob/main/tasks/track_memory_allocations/README.md)).
  - "Allocation counts are identical on all platforms": the task pins mimalloc and excludes arena chunk operations. Byte metrics get a tolerance (1024 bytes for arena size; 1%, at least 4 kB, for the others), because type layout differs between x86_64 and aarch64.
  - "Minification Size" gates output size the same way.
- **CodSpeed memory instrument.**
  - Uses eBPF, runs the benchmark once, and reports peak physical memory (RSS; Linux 6.8 or later, which GitHub's `ubuntu-22.04` and `ubuntu-24.04` meet), peak allocated memory, total allocated, average size and allocation count.
  - Supports glibc, musl, jemalloc, mimalloc and tcmalloc, dynamically or statically linked.
  - Limits: it "works best for small benchmarks with <2M allocations"; concurrent threads make counts and peaks vary; and allocations in spawned subprocesses are tracked only with dynamically linked allocators ([memory](https://codspeed.io/docs/instruments/memory)).
  - Oxc and Ruff run it on pull requests.
- **gungraun** sets limits on DHAT metrics with `--dhat-limits` and runs DHAT in heap, copy or ad-hoc mode ([dhat.md](https://github.com/gungraun/gungraun/blob/main/docs/src/dhat.md)).
- **divan** counts allocations per benchmark through `AllocProfiler`, but cannot compare runs (§1.1).

### 2.2 Peak memory and startup of whole processes

- **hyperfine 2.0.0** (2026-10-07, a breaking release).
  - It now measures wall-clock time and peak memory by default. `--metrics` adds CPU time, instructions, cycles, cache misses and branch misses from hardware counters, on Linux and macOS but not Windows, and "not available when a shell is enabled".
  - Commands now run directly, without a shell (`--shell=none`). The JSON export moved to schema version 2.
  - By default it runs "*at least* 10 benchmarking runs", aiming at about 3 seconds ([release](https://github.com/sharkdp/hyperfine/releases/tag/v2.0.0), [README](https://github.com/sharkdp/hyperfine)).
- **Hardware counters on GitHub-hosted runners.** [actions/runner-images#11789](https://github.com/actions/runner-images/issues/11789) (2025-03) reports that `perf stat` does not work on standard Linux runners, arm64 included, and the issue was closed without a fix.
- **Deno's bench job** runs on GitHub-hosted `ubuntu-24.04` on `main` and for pull requests labelled `ci-bench`. It measures:
  - execution time with hyperfine (`--warmup 3`);
  - peak memory with GNU `time -v`;
  - system-call and thread counts with `strace`;
  - binary and library sizes, and language-server latencies.
  It then pushes the results to the `denoland/benchmark_data` repository ([ci.ts](https://github.com/denoland/deno/blob/main/.github/workflows/ci.ts), [tests/bench/main.rs](https://github.com/denoland/deno/blob/main/tests/bench/main.rs)). No threshold appears in the job.
- **rustc-perf** tracks instruction count, max RSS and binary size. Its triage guide warns that "max rss is much more variable than instruction count" ([triage README](https://github.com/rust-lang/rustc-perf/blob/master/triage/README.md)).

### Fit for Goodfetch

- **Allocation snapshots.**
  - Oxc's exact snapshot is the pattern closest to STYLE.md's `insta` snapshots.
  - It counts identically across platforms only with a pinned allocator, and ADR 0024 already pins mimalloc.
  - Counts must leave out what varies with timing: Tokio worker scheduling, the governor of ADR 0040, and concurrent Sessions. CodSpeed reports the same variance for multi-threaded code.
- **dhat-rs.**
  - Its advice to put each heap test in its own integration-test file conflicts with STYLE.md's "one integration-test binary per crate"; the alternative is `--test-threads=1`.
  - Its maintenance status is a risk.
  - It replaces mimalloc, so it measures counts, not the shipped allocator's footprint.
- **Measuring the shipped binary.** Peak memory and startup of the `goodfetch` binary are process-level metrics. ADR 0025 already has a "startup and binding overhead" family on the reference host. hyperfine 2.0 gives wall time and peak RSS per run on any runner. Instruction counts from hardware counters depend on runner support, which issue #11789 says is missing on GitHub-hosted Linux.

---

## 3. Protocol and SDK overhead

### 3.1 Facts

- **Goodfetch's own numbers.** ADR 0024 measured a small call through the binary at 1.6–2× its in-process cost: medians of 54 against 26 µs in Node and 120 against 99 µs in Python. Ticket #272 cites about 28 µs per small call in process.
- **CodSpeed's limits across processes.**
  - CPU simulation "currently does not follow child processes". `codspeed exec` in simulation "does not currently support statically linked executables" ([CLI](https://codspeed.io/docs/cli)).
  - Simulation excludes system calls, which is where pipe and socket I/O sits ([CPU simulation](https://codspeed.io/docs/instruments/cpu)).
  - Wall time includes system calls and I/O, and whole commands can be benchmarked from a `codspeed.yml` file "including process startup" ([CLI commands](https://codspeed.io/docs/benchmarks/cli-commands)).
- **CodSpeed's Node.js simulation runs unoptimized JavaScript.**
  - Its V8 flags for simulation are `--no-opt`, `--no-maglev` on Node 24 and later, `--predictable`, `--predictable-gc-schedule` and fixed hash and random seeds. The source says Maglev "cuts their instruction count ~4x" ([introspection.ts](https://github.com/CodSpeedHQ/codspeed-node/blob/main/packages/core/src/introspection.ts)).
  - The docs list Node 22 and 24 as needing `@codspeed/*` 6.0.0 or later ([Node.js overview](https://codspeed.io/docs/benchmarks/nodejs/overview)). On 2026-10-13, npm's `latest` is 5.7.1 (2026-06-24), and 6.0.0 exists only as `6.0.0-beta.2` (2026-09-02) (npm registry).
- **Python.** pydantic measures Python calls into Rust in one process with `pytest-codspeed` in simulation mode (§1.2).
- **Deno's language server** speaks JSON-RPC over stdio, like an Attached engine.
  - Its bench job times scripted sessions end to end: startup and shutdown, edits to an 8,000-line file, code lens, and find-and-replace across files ([tests/bench/lsp.rs](https://github.com/denoland/deno/blob/main/tests/bench/lsp.rs)).
  - It also records system-call and thread counts (§2.2).
  - It tracks these numbers but sets no gate.
- **gRPC** benchmarks every language as client and server, and each language against a C++ peer.
  - Scenarios include "contentionless latency – the median and tail response latencies seen with only 1 client sending a single message at a time" and throughput with 64 channels of 100 outstanding messages.
  - It runs "every few hours against the master branch" on a dedicated GKE cluster of mostly 8-core machines, with results published on a public results page ([benchmarking guide](https://grpc.io/docs/guides/benchmarking/)).
- **Rustls's `ci-bench`** runs each scenario in two modes: instructions under Callgrind, and wall time with "100 iterations to deal with noise" recommended ([README](https://github.com/rustls/rustls/blob/main/ci-bench/README.md)).
  - Per Bencher's case study, in instruction mode a custom async runtime makes I/O blocking, so tasks complete in a single poll. Wall-time mode runs on a bare-metal server with turbo, SMT, ASLR and the NMI watchdog off.
  - The case study also says it compares each approved pull request with `main` using a delta-IQR threshold, and comments the result ([case study](https://bencher.dev/learn/case-study/rustls/)).
  - The case study notes that instruction counts "can't disambiguate changes that use the same number of instructions but lead to wildly different wall-time performance".

### Fit for Goodfetch

- **Two halves of an SDK call.** An SDK call to an Attached engine has an in-process half and a cross-process half.
  - The in-process half (building and parsing JSON-RPC messages in the SDK and in the engine) can be measured with instruction counts on Linux: `pytest-codspeed` for Python, gungraun or CodSpeed simulation for Rust. For Node, CodSpeed's simulation measures JavaScript with the optimizing compilers off, and its stable release does not yet document Node 22–26, the lines ADR 0024 supports.
  - The cross-process half (pipe or socket I/O, scheduling, the round trip) falls outside CPU simulation. It needs wall time on bare metal: macro runners, Bencher's runners, or the reference host at release as ADR 0025 already plans.
- **Batching budgets.** "Coarse or batched" operations (ADR 0024) suit deterministic counts, as Oxc does for allocations and Deno for system calls. Examples are protocol messages, bytes and round trips per scenario, or engine system calls per call. Such counts are exact on any runner and fit `insta` snapshots.
- **Static musl build.** The Linux release binary is a fully static musl build (ADR 0024), which `codspeed exec` simulation does not support. A glibc development build of the same code is what instruction-count tools would see.

---

## 4. Console

### 4.1 Facts

- **Lighthouse CI.**
  - `@lhci/cli` 0.15.1 (2025-06-25) bundles Lighthouse 12.6.1, while Lighthouse is at 13.5.0 (2026-09-18) (npm registry). [lighthouse-ci#1136](https://github.com/GoogleChrome/lighthouse-ci/issues/1136), "Lighthouse 13 support?", has been open since 2026-04-06 with 22 upvotes, and the repository's last push was 2026-03-27.
  - `numberOfRuns` defaults to 3.
  - Assertions default to `{"aggregationMethod": "optimistic", "minScore": 1}`.
  - `lighthouse:recommended` "warns when metric values drop below a score of 90" and "disables hard failures for flaky audits".
  - A `budget.json` can be used instead of assertions ([configuration.md](https://github.com/GoogleChrome/lighthouse-ci/blob/main/docs/configuration.md)).
  - User pains: [#734](https://github.com/GoogleChrome/lighthouse-ci/issues/734) (`NO_FCP` failures on the third run).
- **Lighthouse's own variability guide.** "Free CI environments and 'burstable' instances are typically quite volatile". It asks for at least 2 dedicated cores (4 recommended) and no concurrent runs, and says "the median Lighthouse score of 5 runs is twice as stable as 1 run" ([variability.md](https://github.com/GoogleChrome/lighthouse/blob/main/docs/variability.md)).
- **Web Vitals.**
  - The thresholds are LCP within 2.5 s, INP of 200 ms or less and CLS of 0.1 or less, at the 75th percentile of page loads.
  - Lighthouse cannot measure INP in the lab and uses Total Blocking Time as its proxy ([web.dev, updated 2024-10-31](https://web.dev/articles/vitals)).
  - `web-vitals` is at 6.2.3 (2026-10-05).
- **size-limit 14.2.0** (2026-10-07).
  - "Checks every commit on CI ... and throws an error if the cost exceeds the limit", Brotli by default. A GitHub Action comments size changes on pull requests.
  - The `time` plugin runs headless Chrome. Its "measurements depend on available resources and might be unstable" ([README](https://github.com/ai/size-limit)).
- **Vite 8.3.4** warns, but does not fail, when a chunk exceeds `build.chunkSizeWarningLimit`, which defaults to 500 kB uncompressed ([build options](https://github.com/vitejs/vite/blob/main/docs/config/build-options.md)).
- **Render profiling in tests.**
  - React's `<Profiler onRender>` reports `actualDuration` and `baseDuration` for each commit. "Profiling adds some additional overhead, so it is disabled in the production build by default" ([react.dev](https://react.dev/reference/react/Profiler)).
  - Reassure 1.6.0 (2026-07-22) measures render duration and render count. It repeats each scenario (10 runs by default), applies a significance test, and reports changes in count separately. It needs Jest with React Testing Library or React Native Testing Library ([README](https://github.com/callstack/reassure)).
  - Its maintainers ran it under Vitest and found results "much more unsteady than under Jest", and answered "No" to plans for Vitest support ([reassure#421](https://github.com/callstack/reassure/issues/421)).
- **Browser flows.**
  - CodSpeed's Playwright plugin measures wall time only and "today ... supports Electron apps as a target"; browser targets are "on the roadmap" ([Playwright plugin](https://codspeed.io/docs/benchmarks/nodejs/playwright)).
  - Playwright exposes raw CDP through `CDPSession` in Chromium ([docs](https://playwright.dev/docs/api/class-cdpsession)).
  - Vitest 5.0.4 runs benchmarks on tinybench 6.2, and `@codspeed/vitest-plugin` reports them to CodSpeed.

### Fit for Goodfetch

- **The Console bundle and ADR 0057.** The Console's assets are embedded in the binary, brotli-compressed (0.40 MB for the prototype), so they already count against the Size budget. A size-limit entry per Console chunk would catch growth earlier, at the source of it, in the pnpm workspace.
- **Lab metrics.**
  - The Console is served from loopback by the Daemon. Field Web Vitals do not exist for it, so only lab metrics (LCP, CLS, TBT) apply, and these are the noisy measurements Lighthouse warns about on free CI.
  - Lighthouse CI's lag behind Lighthouse 13 is a maintenance risk.
- **Render counts.**
  - Counts of renders and commits under `<Profiler>`, or Reassure's "Changes To Count", are deterministic and suit a per-change gate. Render durations are not.
  - STYLE.md runs TypeScript tests with Vitest, and Reassure is Jest-only.

---

## 5. Gating without retries

### 5.1 Facts

| Method | Who | How it decides | Source |
|---|---|---|---|
| Fixed percentage against the base branch | CodSpeed (10% default); github-action-benchmark (`alert-threshold` 200% by default, `fail-on-alert` off); gungraun soft limits | One comparison of head against base | [CodSpeed](https://codspeed.io/docs/features/customization), [action.yml](https://github.com/benchmark-action/github-action-benchmark/blob/master/action.yml) (v1.22.2, 2026-09-15) |
| Absolute ceiling | gungraun hard limits; size ceilings (ADR 0057) | Fails above a number, whatever the base | [regressions.md](https://github.com/gungraun/gungraun/blob/main/docs/src/regressions.md) |
| Exact snapshot | Oxc "Allocations" and "Minification Size" | Any change fails until the snapshot is updated in the same pull request | [ci.yml](https://github.com/oxc-project/oxc/blob/main/.github/workflows/ci.yml) |
| Statistical test against history | Bencher (t-test, z-score, log normal, IQR, delta IQR) | Alert when a new metric falls outside a limit computed from past metrics; the z-score test wants at least 30 | [thresholds](https://bencher.dev/docs/explanation/thresholds/) |
| Outlier fences on past changes | rustc-perf | A change is significant above Q3 + 3×IQR of historical relative changes; magnitude and relevance then decide whether it is reported | [comparison-analysis.md](https://github.com/rust-lang/rustc-perf/blob/master/docs/comparison-analysis.md) |
| Change-point detection | MongoDB (E-Divisive means); Hunter, now Apache Otava 0.8.0-incubating (2026-05-08) | Finds persistent shifts in a series after merge. MongoDB reports a "dramatically dropped" false-positive rate against thresholds. Hunter replaced random permutations with a Student's t-test for deterministic results. Otava "adapts automatically to the level of noise" and can compare a branch with main. | [Daly et al. 2020](https://arxiv.org/abs/2003.00584), [Hunter 2023](https://arxiv.org/abs/2301.03034), [Otava](https://github.com/apache/otava) |
| Same-machine pairing or interleaving | Heisler's pairs; Laaber et al.; Duet; tango | Base and head on the same machine, interleaved or concurrent, then a rank-sum test or overlapping bootstrap intervals | §1.3 |
| Deterministic metric | SQLite ("repeatable to 7 or more significant digits" with Cachegrind, against "scarcely ... one significant digit" for wall time); rustc-perf; Rustls; CodSpeed simulation | No statistics needed; layout and environment effects remain | [sqlite.org/cpu.html](https://sqlite.org/cpu.html) |

Also:
- **Acknowledging a regression.** CodSpeed lets an admin acknowledge a regression so the check passes, or can be set to send an informational status instead of a failure ([performance checks](https://codspeed.io/docs/features/performance-checks/index)).
- **Merge queues.** We found no CodSpeed documentation of GitHub's `merge_group` event. Its documented triggers are `push` and `pull_request`.

### Fit for Goodfetch

- **What ADR 0025 already decides.** Its rules fix each claim's minimum effect Δ before measuring, use block-bootstrap intervals, and give each family a release-gate Δ set from the reference host's measured spread. Those are the statistics for the release tier.
- **What a per-change tier could add.** On shared runners, the facts above leave deterministic metrics checked by snapshot or percentage. Instruction counts, allocation counts, protocol message counts and sizes each fail a pull request on their first run, with no retry, as Oxc's checks do.
- **Wall-time series on `main`.** Nightly results or results from the reference host form a series that change-point detection reads, without blocking any one pull request.
- **Merge queue.** ADR 0032's merge queue would need either a gate that runs on `merge_group` events or a gate on the pull request whose base is close enough to `main`. CodSpeed does not document the first.

---

## Unverified

- **Vendor's own figures.** The CodSpeed noise figures (2.66% against 0.56%, the false-positive model) come from CodSpeed, which sells the Macro Runners. The two environments also differed in architecture: x86-64 GitHub runners against Graviton arm64. We found no independent replication on GitHub-hosted runners.
- **Same CPU on 8-vCPU runners.** That 8-vCPU larger runners always get the same CPU rests on CodSpeed's one experiment. GitHub documents no such guarantee.
- **Hardware counters on GitHub-hosted Linux runners.** The only source is a 2025-03 issue closed without a fix. Current behaviour was not tested.
- **Whether benchmark checks are required.**
  - Rulesets were read from `GET /repos/{owner}/{repo}/rules/branches/main`. Classic branch protection is not visible without admin rights, so a required CodSpeed check in Ruff, Biome or pydantic cannot be excluded.
  - For uv, Ruff and Oxc, the conclusion that benchmarks do not block rests on their visible required-checks jobs and rulesets.
- **Rustls details.** The bare-metal tuning, the delta-IQR threshold and the pull-request workflow come from Bencher's case study, a vendor page. Only the two measurement modes were confirmed in Rustls's own `ci-bench` README.
- **Deno's bench job sets no gate.** This is inferred from the job's steps, in which no comparison step appears.
- **gungraun and static binaries.** Whether gungraun's binary benchmarks run correctly on a fully static musl binary under Valgrind was not checked.
- **Valgrind's slowdown factor and CI time cost** for Goodfetch-sized benchmarks were not measured.
- **CodSpeed and merge queues.** No documentation was found either way.
- **CodSpeed walltime on Node 22–26.** The docs table lists `@codspeed/*` 6.0.0 for Node 22 and 24, and nothing for Node 26. Whether walltime mode works on 5.7.1 with those lines was not tested.
- **The Laaber et al. study** used Java and Go on 2018-era instances of AWS, GCE and Azure, not GitHub's runners.
