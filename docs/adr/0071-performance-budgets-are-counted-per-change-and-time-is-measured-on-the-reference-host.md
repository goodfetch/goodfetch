# Performance budgets are counted on every change, and time is measured only on the reference host

Every change is held to a **Performance budget** made only of figures that repeat exactly on a shared runner: instruction counts and heap metrics under Valgrind, compared with the change's base in the same job; counts of protocol messages, bytes and renders, as snapshots; and the Console's bundle bytes. Resident memory has ceilings. Wall time never fails a change: it is measured every night on `main`'s head on the reference host, as the **Performance series**, once the owner approves that use, and at release by ADR 0025's benchmark, whose Δ rule is unchanged.

ADR 0025 measures performance on the reference host at each release candidate, and ADR 0057 holds sizes on every change. Nothing held the speed or memory of the code between releases, so a regression would surface weeks after the change that made it. The facts limit what a per-change gate can hold:
- **Time on shared runners is noise.** GitHub-hosted runners vary by a 2.66% coefficient of variation on real benchmark suites, so a 2% gate gives about 45% false alarms (CodSpeed's own measurement), and one runner label hands out different CPUs. ADR 0061 already says "No run fails for being slow within its cap".
- **What projects require is exact counts.** Oxc's required checks are an allocation snapshot and a minified size; rustc-perf, Rustls and SQLite rely on instruction counts under Valgrind. Instruction counts of one binary on one machine repeat exactly, and move by 0.1–0.5% across CPUs.
- **CodSpeed is common but nobody visibly blocks on it** (uv, Ruff, Biome, Oxc, pydantic), and Polars removed it.
- **Counts are blind to some costs:** system calls, I/O, cache behaviour across threads, V8 and the SDK-to-engine round trip. Those need wall time on a quiet machine.

Decided in [Performance regression testing](https://github.com/Jbmanllr/automan/issues/284) ([grilling record](https://github.com/Jbmanllr/automan/issues/284#issuecomment-6154117858)), with the agent answering on the owner's delegation. The facts come from [Performance regression testing in CI](https://github.com/Jbmanllr/automan/issues/272) ([findings](https://github.com/Jbmanllr/automan/blob/research/performance-testing/rnd/research/2026-10-13-performance-testing.md)) and [Rust test tooling in 2026](https://github.com/Jbmanllr/automan/issues/263). It builds on ADRs 0061 and 0062, on ADR 0064's Live checks and results store, and on the Console test stack of [Console and docs test stack](https://github.com/Jbmanllr/automan/issues/282), in review as ADR 0068. The rules every test follows are in `TESTING.md`.

## Decision

### What may fail a change

- **Only counted figures gate a change.** No check on a GitHub-hosted runner compares a duration with a threshold.
- **The counted figures:**
  - instructions (Callgrind's `ir`) per benchmark;
  - heap metrics per benchmark (DHAT): total allocations, total bytes, and bytes at the global maximum;
  - protocol messages, answers, events and their bytes per scenario, for every surface;
  - React commits and DOM elements per Console action;
  - bytes of the Console's and the TypeScript packages' entry points, brotli-compressed.
- **Resident memory has ceilings,** not per-change thresholds.

### Instruction and heap benchmarks

- **Tool: gungraun** (formerly iai-callgrind), on Valgrind, pinned. It runs library and binary benchmarks under Callgrind and DHAT, compares with saved baselines, and exits with code 3 on a regression. CodSpeed is not adopted: its verdict would live in a vendor service outside review.
- **Where they live**, beside the code (ADR 0062). A crate with performance-sensitive code has:
  - `benches/scenarios.rs`, the scenario functions;
  - `benches/instructions.rs`, gungraun under Callgrind, with mimalloc as the global allocator, as the binary ships (ADR 0024);
  - `benches/heap.rs`, gungraun under DHAT, with the system allocator, because DHAT counts allocations by replacing `malloc`, which mimalloc does not call. The number of Rust allocations does not depend on the allocator;
  - `benches/wall.rs`, criterion, for the Performance series only.

  Scenarios that cross crates (startup, a protocol round trip) live in the binary crate. Benchmarks are not tests and add no integration-test binary.
- **Startup** is two binary benchmarks in the binary crate: an Attached engine started over stdio, answering `hello`, and exiting at end of input; and `goodfetch --version`.
- **Build:** Cargo's `bench` profile, which is the release settings with thin LTO and `debug = "line-tables-only"` (gungraun needs symbols), on `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`. Valgrind runs on neither Windows nor Apple-silicon macOS, so the counts come from Linux. The gnu build's counts stand for the static musl release binary, which the release benchmark measures.
- **A benchmark measures one pinned scenario:** inputs from a literal, a fixture or a seed, no network, no sleep, no work whose amount depends on timing, and a current-thread Tokio runtime where it needs one.
- **Admission.** A pull request that adds a benchmark or changes its body runs it 20 times on one build. The spread of every gated figure must stay under 0.5%, a tenth of the threshold, or the benchmark is fixed before it gates. A benchmark whose body changes gets a new ID, so a base and a head never compare different work; a benchmark with no base is reported, never gated.

### Comparison with the base, in the same job

- **`xtask perf` builds and measures the base, then the head, in one job on one runner:** the merge base in the pull-request tier, the group's base in the merge queue. The base is saved as gungraun's `base` baseline, and the head runs with `--baseline=base`. The runner's CPU then cancels out.
- **The base build reuses `main`'s cache.** The cache-warming job of ADR 0061 also builds the benchmark targets on every push to `main`, with `Swatinem/rust-cache`'s `cache-workspace-crates` on, since by default it caches only dependencies and the base's own crates would be rebuilt in the job.
- **`xtask affected` selects** the bench targets of changed crates and of the crates that depend on them.
- **Threshold: +5%** on instructions and on each heap metric, per benchmark. It is ten times the drift seen across CPUs, and still catches a 5% regression in one change. Improvements are reported, never gated.
- **Accepting an increase.** A pull request that trips the threshold passes when its body has a **"Performance change"** section listing each tripped benchmark with its measured change and its reason, and the fresh review accepts that section explicitly on the pull request's exact head. This is ADR 0057's rule for a size increase.
- The thresholds are constants of `xtask perf` that cite this ADR. They change only by changing it.

### Counts kept as snapshots

These are exact by construction, so they are ordinary tests with snapshots (`TESTING.md`'s Snapshots rules). A changed count is a snapshot change, reviewed as a change of expected behaviour.

- **Protocol counts.** For a named set of scenarios per surface (each SDK's high-level calls, the CLI's generated commands, MCP tool calls), the requests sent, the answers and events received, and their bytes after the shared redactions:
  - Rust, in the binary crate's `tests/it`, against the real binary;
  - TypeScript and Python, in their unit tests, against the Scripted engine of ADR 0069 (in review in [Decide the SDK and CLI test stack (ADR 0069)](https://github.com/Jbmanllr/automan/pull/304)), which records the requests it receives.
- **MCP:** the bytes of `tools/list` and of each tool result in those scenarios. For an agent, bytes are context.
- **Python import cost:** the set of modules `import goodfetch` loads.
- **Console render budgets,** in Vitest Browser Mode component tests (ADR 0068): named heavy screens are wrapped in React's `<Profiler>`, and the test snapshots the number of commits per action (an event batch on the Runs screen, opening a run, filtering) and the number of DOM elements a 10,000-row list renders.

### Bundle bytes

- **size-limit** (brotli) measures the built Console, and the entry points of the TypeScript SDK, adapters and Integrations with what each imports.
- **Ceilings:** the whole embedded Console stays under **1 MB** brotli, 2.5 times the prototype's 0.40 MB; it also counts against the binary's Size budget. The Console's initial route (what loads before the Runs screen's first render) gets a ceiling from area 12's first Console build at 1.25 times its measured size, approved by the owner. A ceiling moves only with the owner's approval.
- **Increase:** `xtask perf` compares each entry with the base build in the same job. Growth over **5 kB** brotli per entry needs the "Performance change" section.

### Memory beyond the heap

- **Resident-memory ceilings,** checked in the Release suite on every OS with the shipped binary, for three scenarios:
  - an Attached engine idle after `hello`;
  - a Daemon idle with no run;
  - an HTTP crawl of 1,000 local Proving ground pages at the default concurrency.

  Each OS has its own ceiling per scenario. The ticket that builds the check sets it at 1.5 times the highest peak of 20 runs, rounded up, with the owner's approval, and only the owner moves it. The 50% margin is far above run-to-run spread (rustc-perf calls max RSS "much more variable than instruction count"), so the check gives one verdict for one build.
- **The Console's leak check:** a served Playwright test (ADR 0068) streams 10,000 events into the Runs screen. The JS heap after a forced garbage collection, read over CDP, is within 10% of its value after the first 1,000 events. A ratio, so the machine does not matter.

### The Performance series

- **Every night, on `main`'s head, on the reference host** of ADR 0025:
  - criterion's wall time for every scenario (`benches/wall.rs`);
  - each SDK's small-call round trip against an Attached engine;
  - startup wall time and peak memory of the binary, with hyperfine 2.0;
  - the Console's lab metrics (LCP, CLS, TBT) with Lighthouse 13, driven directly against the binary-served Console, the median of 5 runs against the "good" values: LCP ≤ 2.5 s and CLS ≤ 0.1 from Web Vitals, and TBT ≤ 200 ms from web.dev's TBT guidance (TBT is not a Web Vital; lab tools use it in place of INP). Lighthouse CI is not used: it still bundles Lighthouse 12.
- **It is a paid resource, gated on the owner.** It starts only once the owner approves the reference host's nightly use. Until then it runs nowhere, and the counted gates stand alone. It uses no hosted minutes, so it runs while the repository is private and the nightly tier is off, an exception to ADR 0032's rule like ADR 0064's test-health job.
- **It is a Live check** (ADR 0064). It runs through `bench/`'s campaign runner under the reference host's gates, and declares in `bench/` before its first run:
  - **the control:** a pinned binary named in the declaration. Before the first release it is the build of a `main` commit named when the owner approves the series; from the first release on, the last release. The control changes only by a declared change in `bench/`, which starts a new series;
  - 10 trials per figure each night, interleaved in random order between `main`'s head and the control;
  - the night's figure is the ratio of the head's median to the control's, so a change in the machine moves both;
  - the statistic is Apache Otava's change-point detection over the series of nightly ratios, at its default significance, reporting a lasting shift of 5% or more.

  Otava needs several nights after a change to confirm it, more for a smaller shift ([its basics](https://github.com/apache/otava/blob/main/docs/BASICS.md)), and then dates it to the night it began. So a lasting 5% shift is reported a few nights late, not the next morning; the declaration states the expected number of nights, measured on the series' first month.

  A confirmed shift opens or updates the series' one `live-check` issue (ADR 0064), which lists each shifted figure with the night the shift began and the commits since the night before; ADR 0064's two-day rule applies. The series never fails a pull request or a merge, and its figures are never published as benchmark results.
- **Per-change wall time on bare metal** (CodSpeed's or Bencher's runners) is not adopted for V1. The series' record is the evidence to reopen it.

### Relation to the benchmark and to releases

- **ADR 0025 is unchanged.** Counts and the series are early signals. They never relax or replace a family's Δ, and no count alone blocks a release. The startup and binding overhead family keeps its release-gate claim for the round trip.
- **Drift check.** With the daily test-health job (ADR 0064), a check compares `main`'s head with the last release's counted figures in the results store. A figure up more than 10% since that release, or a resident-memory peak within 10% of its ceiling, opens or updates one `needs-triage` issue per figure. It catches many accepted steps under 5%. It only reads the results store, so it fits the test-health job's 15-minute timeout and its ceiling of included minutes. It compares nothing before the first release; until then the figures of `main`'s head are stored by the manual-dispatch Release runs, and then by the merge queue.
- **Release notes** list each counted figure that changed by more than 1% since the previous minor, as ADR 0057 lists sizes.

### Suites and commands

| Check | Suite | Platforms |
|---|---|---|
| Benchmarks compile (Clippy `--all-targets`) | Quick | every |
| Count snapshots in unit tests (Scripted engine, the Console's `unit` project) | Quick | every |
| Count snapshots against the binary; render and DOM counts; the Console's leak check; the Python import set | Pre-merge | Linux x64; every cell in Release |
| `xtask perf`: instructions, heap and bundle bytes against the base | Pre-merge | Linux x64; Release adds Linux arm64 |
| Benchmark admission (20 runs) | Pre-merge, when a benchmark is added or changed | Linux x64 |
| Resident-memory ceilings | Release | every OS |
| Drift check | none: the daily test-health job | Linux |
| Performance series | none: a nightly Live check, after the owner's approval | reference host |

- **`just perf [base]`** runs `xtask perf` over what changed, against the merge base with `origin/main` or the given base, with CI's thresholds. On Linux it runs natively. Elsewhere it runs in the pinned Linux CI image through a container engine; Valgrind supports ARM64 Linux, so Apple-silicon machines run it. Without a container engine it lists itself as skipped, with the reason (ADR 0061). `just pre-merge` includes it.
- **The comparison job counts against the Pre-merge budget** (each job ≤ 15 min) and shards by bench target when it would not fit.

### Reports

- `xtask perf` writes gungraun's and size-limit's JSON, posts one table per run on the job's summary page (figure, base, head, change, verdict), and uploads the files with the JUnit artifacts. The results store keeps every figure from the merge queue, `main` and the series, and the drift check and the series read them there.

## Considered Options

- **Wall time with a threshold on GitHub-hosted runners**, per change or per suite: rejected. At 2.66% noise, a useful threshold is flaky, and ADR 0061 and the no-retry rule forbid flaky gates.
- **CodSpeed** (simulation, memory, macro runners): rejected as the gate. The verdict sits in a vendor service outside review, merge queues are undocumented, admins acknowledge regressions in its interface, and its Node simulation turns V8's optimizing compilers off. gungraun gives the same Callgrind counts in CI the project owns.
- **criterion or divan on shared runners:** rejected for gating. Criterion's FAQ advises against trusting cloud CI results, and divan cannot compare runs.
- **Committed snapshots of instruction counts** (Oxc's pattern, applied to instructions): rejected. Counts move by 0.1–0.5% across runner CPUs, so an unchanged commit would fail. Snapshots are kept for figures exact by construction.
- **A baseline from `main`'s last run:** rejected. It was measured on another machine; the same-job base costs one more measuring pass and removes that.
- **A 2% or a 10% threshold:** rejected. 2% trips on code-layout effects, which Oxc saw at 3–5%; 10% lets a real 9% regression through each change.
- **dhat-rs heap tests or a hand-written counting allocator:** rejected. dhat-rs is self-described experimental and wants one test per file; gungraun's DHAT mode already counts, in the same job and with the same thresholds.
- **Resident memory with a per-change threshold:** rejected. Resident memory varies with scheduling; generous ceilings catch blow-ups without flakiness.
- **Instruction counts for the TypeScript and Python SDKs:** rejected. Node's JIT under Valgrind is neither stable nor representative, and Python's in-process instrument needs CodSpeed. Their costs are what crosses the wire (counted) and the round trip (timed).
- **Per-change wall time on vendor bare metal** (CodSpeed macro runners, Bencher): not adopted for V1. It is paid beyond small free tiers, puts a vendor in the merge path, and fork pull requests need extra workflows; the series shows a lasting regression within a few nights.
- **Per-change wall time on the reference host:** rejected. ADR 0032 keeps pull-request code off it.
- **Lighthouse CI, size-limit's time plugin, Reassure:** rejected. Lighthouse CI bundles Lighthouse 12; size-limit calls its time plugin possibly unstable; Reassure is Jest-only and its authors found Vitest results unsteady.
- **Only the release benchmark:** rejected. Weeks pass between a regression and its discovery, across many changes to bisect.

## Consequences

- **Amends ADR 0032**, at two spots, each with a note:
  - the reference host is reached by tags, manual dispatch and, once the owner approves, a nightly schedule on `main`'s head, which runs only merged code. It still never runs pull-request code;
  - the Performance series runs while the repository is private, though the nightly tier is off then, since it uses no hosted minutes.
- **Adds to ADR 0025:** the Performance series uses the reference host and `bench/`'s runner, and its figures are never published as benchmark results. The release gate and every Δ are unchanged.
- **Adds to ADR 0057:** the Console's bundle has its own ceiling and per-change increase check, ahead of the binary's.
- **Adds to ADR 0061:** the cache-warming job builds the benchmark targets and caches workspace crates; `xtask affected` selects bench targets; `just perf` joins the recipes and `just pre-merge` runs it; the Performance budgets are those its time budgets exclude.
- **Adds to ADR 0064:** the Performance series joins its table of Live checks (nightly on the reference host, blocking nothing directly, owned by area 2); the drift check runs with the daily test-health job; performance figures join the results store.
- **Adds to ADR 0068 (in review):** render and DOM counts in component tests, and the leak check in served tests.
- **Adds to ADR 0030's release notes:** each counted figure that changed by more than 1% since the previous minor.
- **ADR 0037's area table** lists this ADR in the rows of area 1 (the job, `xtask perf`, `just perf`, the drift check), area 2 (the series in `bench/`), area 3 (the engine's scenarios and resident-memory ceilings), area 11 (the SDK, CLI and MCP counts) and area 12 (the Console's budgets).
- **Changes area 1's spec** ([Spec: Repository, CI and release (V1 area 1)](https://github.com/Jbmanllr/automan/issues/130)): the pull-request and merge-queue tiers run `xtask perf`; Cargo gets a `bench` profile beside `dev`, `test` and `ci-test`; the CI image carries Valgrind and gungraun's runner; the reference host's runner also takes the nightly schedule once approved; the release notes gain the counted figures.
- **Changes area 2's spec** ([Spec: Proving ground and conformance (V1 area 2)](https://github.com/Jbmanllr/automan/issues/152)): `bench/` declares and runs the Performance series as a campaign, with Apache Otava for change points.
- **Area 3's spec** ([Spec: Engine runtime (V1 area 3)](https://github.com/Jbmanllr/automan/issues/213)) gains benchmark scenarios for its hot paths (framing, dispatch, run-state writes, admission), the startup benchmarks and the three resident-memory scenarios.
- **Area 12's spec** sets the initial route's ceiling and names the screens with render budgets.
- **Owner approvals:** the reference host's nightly use, every resident-memory ceiling, and the Console's initial-route ceiling.
- **The "Test infrastructure" spec** builds `xtask perf` and `just perf`, the comparison job and its base build, the admission run, the drift check, the reports, and the resident-memory check.
