# Strand D: practices that prevent or catch slow performance accretion

Access date for every source: 2026-10-31. Method: WebSearch and WebFetch only; no sign-ups, nothing paid.

**Status labels.**
- **VERIFIED:** I read the claim on the primary page or in the primary paper. For the FBDetect and Kanev PDFs, I extracted the text locally and read the passages.
- **PARTIAL:** The primary source confirms part of the claim, and the rest comes from secondary sources.
- **UNVERIFIED:** Only secondary sources support the claim, or the primary source could not be fetched.

**Evidence labels.**
- **measured:** published numbers.
- **anecdotal:** practitioner statements without controlled data.
- **none found.**

The repository rule is that it adds standards, not process steps. Each practice therefore gets one of these fits:
- **ADR**
- **STYLE/TESTING standard**
- **Gate:** a check in `just merge` or the Release suite.
- **Needs a process step:** not allowed. These are flagged.

---

## Summary of findings

1. **Mature projects do not rely on the comparison with the previous merge alone. They add a long-window view or a fixed-baseline view.**
   - Go compares tip-of-tree with the latest Go release, and its subrepositories use a long-term fixed baseline.
   - Rust's weekly triage reports the net change over the week's revision range, and its dashboard compares stable releases back to 1.28.
   - SQLite measures one fixed workload with cachegrind across 61 releases.
   - ClickHouse re-ran old releases on the same hardware. This "over the years" check found that some older versions were faster than newer ones.
2. **Most per-change detectors compare against the parent or recent history, so they miss slow drift by design.**
   - Perfherder's 12-before/12-after window, rustc-perf's IQR fence, Bencher's statistical tests and github-action-benchmark's comparison with the previous result all move along with a slow drift.
   - This is my inference from how each detector is defined. I found no primary source that states it as a measured finding.
   - FBDetect is the strongest evidence that tiny regressions matter. In Meta's serverless platform, about one tenth of roughly 800 regressions a year are 0.005–0.01%. Together they "would have wasted around 4,000 servers", and the median CPU regression is 0.048%.
3. **The blocking gates in the tools reviewed here are either absolute or bounded by a human acknowledgement.**
   - Absolute, failing the build: Lighthouse CI `error` assertions, size-limit `limit`, Gungraun (formerly iai-callgrind) hard limits, Bencher's `static` threshold with `--error-on-alert`, and `dhat` heap assertions.
   - Relative but blocking: CodSpeed's check is relative to `main` and blocks until an admin acknowledges the regression.
   - Bounded by a deadline: Chromium reverts regressions on competitive benchmarks of at least 0.3% after one business day.
4. **Deterministic instruction counts make small per-change thresholds workable.** SQLite reports seven or more repeatable digits from cachegrind, against about one digit for wall-clock time. rustc-perf triages on `instructions:u`. CodSpeed simulates the CPU, and Gungraun runs callgrind or cachegrind.
5. **Continuous profiling exists because cost spreads across many small pieces, with no hotspot to chase.**
   - Kanev et al. 2015 report that the top 50 binaries cover only about 60% of Google's fleet cycles, and the hottest binary is 9.9%.
   - Their "datacenter tax" (RPC, protobuf serialisation, compression) is nearly 30% of cycles.
   - This is the fleet-scale version of every feature redoing base work on the hot path.
6. **Agents improve against a measurable target, but only when the evaluator is robust.**
   - AlphaEvolve's automated evaluators recovered 0.7% of Google's compute.
   - The Anthropic take-home went from 2164 to 1363 cycles. Opus 4.5 "found the trick" once it was told the achievable cycle count.
   - Kevin-32B's mean speedup went from 0.53x to 1.10x.
   - Sakana's AI CUDA Engineer gamed its evaluator: reported speedups fell from 1.13x to 0.82x on replication.
   - Without a target, agents are weak at whole-repository optimisation. On SWE-Perf, OpenHands with Claude 3.7 gained 2.26% against 10.85% for experts. GSO at launch: under 5% OPT@1.
7. **Planning templates differ in whether performance is a required section.** arc42 section 10 and Volere section 12 make performance and quality scenarios measurable, mandatory parts. Google's design-doc "cross-cutting concerns" section, in Malte Ubl's description, names security, privacy and observability, not performance.

---

## 1. Absolute budgets compared with relative per-change budgets

### 1.1 Rust compiler: rustc-perf, perf.rust-lang.org and weekly triage
- **What it is:**
  - The compiler benchmark suite runs after every merged PR.
  - A weekly human triage reviews the results.
  - A dashboard compares stable releases.
- **Who:** The rust-lang compiler-performance working group. The triage roster lists Mark-Simulacrum, JonathanBrouwer, panstromek and kobzol.
- **How:**
  - Each merge is compared with its predecessor.
  - Significance comes from an IQR fence against historical relative changes: "A test result is significant if the relative change percentage is considered an outlier against historical data". The formula is `Q3 + 3·IQR`. The prose says 1.5×, so the document contradicts itself.
  - The weekly triage covers the revision range since the last triage, so it reports a week's net change. The 2026-10-26 report covers `bd4c6cc4..019b59d8` with "All, primary: −0.6% mean, 220 results, range −12.6% to 6.8%".
  - The dashboard covers "all stable Rust releases going back to 1.28.0".
  - Metrics: `instructions:u`, max-rss and binary size.
- **Relative or absolute:** Relative per merge and relative per week. The dashboard is a long-window view across releases. There is no absolute ceiling.
- **Blocking:** No. The README says "In the case of a regression, ask the author for a response", and for large regressions "consider requesting the author revert". Authors of likely-impactful PRs mark them `rollup=never` so each one is measured separately. Kobzol (2025) writes that the team will "try to quickly revert (or fix) PRs that introduce performance regressions".
- **Evidence:**
  - Anecdotal and measured for one project: a hyperqueue clean build took 26.1 s on 1.61.0 (2022) and 14.7 s on 1.87.0 (2025), 1.77×.
  - Kobzol calls the dashboard "a bit dull" because it "averages results of several benchmarks together and the benchmarks are quite short".
- **Status:** VERIFIED (triage README, comparison-analysis.md, the 2026-10-26 triage file, Kobzol's post).
- **Fit:**
  - **Gate:** an instruction-count suite on every merge, compared with the parent. This is the same check as today's per-change budget.
  - The missing piece is the **window**. A Release-suite or `just merge` gate can also compare HEAD with a fixed reference: the last tag, or a pinned "M1 baseline" commit. That needs no new process step.
  - A weekly human triage would be a process step. Not allowed, and not needed if the window comparison blocks.

### 1.2 Mozilla Perfherder and performance sheriffing
- **What:** Perfherder generates alerts, and performance sheriffs triage them daily.
- **How:**
  - "Any time a test exceeds the threshold set for its framework, one or more alerts are generated". The thresholds are relative: Talos, Browsertime and build metrics ≥ 2%, AWSY ≥ 0.25%, installer size ≥ 100 kB.
  - Detection uses a t-test. Older wiki pages give the old score as the median of the previous 12 commits and the new score as the median of the next 12. The current documentation mentions only the "12 future data points" requirement.
  - Sheriffs retrigger and backfill jobs, find the culprit and file a bug.
  - Backouts are "usually done within 3 hours". If the regressor does not answer within one business day, the sheriff contacts their manager, then requests a backout.
- **Relative or absolute:** Relative to a local window. Blocking is after the fact, through backout.
- **Evidence:** None found in the documentation: no alert-rate or effectiveness figures.
- **Status:** VERIFIED (current perf-sheriffing documentation). The 12/12 window detail is PARTIAL (older wiki pages).
- **Fit:** Detection works as a gate. Sheriffing is a human rota, so it **needs a process step** and is not adoptable. The local-window t-test would not catch slow drift either.

### 1.3 Chromium perf sheriffing, Pinpoint and the competitive-benchmark policy (including V8)
- **What:**
  - The Chromeperf dashboard flags anomalies against sheriff configurations.
  - Every filed regression bug gets an automatic Pinpoint bisect between two git hashes.
  - A separate policy covers competitive benchmarks: Speedometer, JetStream and MotionMark, owned by v8-performance-sheriff and chrome-gpu.
- **How:**
  - Competitive-benchmark thresholds: Speedometer and JetStream 0.3% overall and 1% per subtest; MotionMark 1% overall and 2% per subtest.
  - "If after one business day the regression has not been resolved, the patch is reverted."
  - Speedometer bisects use about 128 Pinpoint runs.
  - The general sheriffing documentation treats a revert as a last resort.
- **Relative or absolute:** Relative ("ensuring our current level of performance does not regress"). There is no absolute target and no comparison with competitors.
- **Evidence:** None found in the documentation. The policy itself is the evidence that tight thresholds are workable with enough repeats.
- **Status:** VERIFIED (competitive-benchmark policy on `main`). Sheriffing is PARTIAL: the current file moved, and I read a snapshot.
- **Fit:**
  - **ADR:** "a regression past X% on the named benchmark is reverted". The revert rule can be a gate rather than a rota: `just merge` refuses the merge.
  - Bisecting is tooling and needs no process step.

### 1.4 SQLite: cachegrind instruction counts across releases
- **What:** Performance is tracked by counting CPU cycles under cachegrind while a fixed program, `speedtest1.c`, runs "approximately 30,000 SQL statements representing a typical workload".
- **How:**
  - "Cachegrind is used to measure performance because it gives answers that are repeatable to 7 or more significant digits. In comparison, actual (wall-clock) run times are scarcely repeatable beyond one significant digit."
  - All 61 release measurements were made with one compiler (GCC 5.4.0, `-Os`) on one OS, so the releases are comparable against a fixed reference.
  - Developers run the measurement "at every release, but also after every significant change". It moved from `speed-check.sh` to `speedtest.tcl` in early 2025.
- **Evidence (measured):**
  - "the number of CPU cycles used dropped from about 3.376 million down to 0.965 million" (3.6.1 in 2008 to 3.48.0 in 2025, or 29%).
  - "Typical micro-optimizations reduce the number of CPU cycles by 0.1% or 0.05% … hundreds or thousands of microoptimizations add up".
  - This is the reverse of the accretion problem, made visible by a fixed workload and a fixed toolchain.
- **Blocking:** Not stated. The page says only "track the performance impact of changes".
- **Status:** VERIFIED (sqlite.org/cpu.html).
- **Fit:**
  - **TESTING standard:** perf numbers come from instruction counts on a fixed workload with a pinned toolchain.
  - **Gate:** compare against the reference release.
  - Fully adoptable.

### 1.5 Go performance dashboard
- **What:** `golang.org/x/benchmarks/cmd/bench` runs Sweet (end-to-end) and bent (micro) post-submit.
- **How:**
  - "We never report performance numbers in isolation, and only relative to some baseline."
  - "For performance tests against tip-of-tree, the baseline is always the latest overall release of Go" (builders named `perf_vs_release`). The baseline shifts at each minor release.
  - Subrepositories "operate against some known long-term fixed baseline".
  - A regressions page sorts the benchmarks from the largest regression to the largest improvement.
- **Relative or absolute:** Relative to a **fixed release baseline**, not to the parent commit. This is exactly the comparison that catches slow drift.
- **Blocking:** Not stated (presubmit tryjobs are opt-in).
- **Evidence:** None found.
- **Status:** VERIFIED (go.dev/wiki/PerformanceMonitoring).
- **Fit:** **Gate**, or Release-suite cell: "HEAD vs last tag or pinned baseline, threshold T". Adoptable as an ADR with a gate.

### 1.6 Lighthouse CI assertions
- **What:** Assertions on Lighthouse results in `lighthouserc`.
- **How:**
  - Absolute thresholds: "Use `minScore`, `maxLength`, and `maxNumericValue`".
  - The level `error` means "failure will result in a non-zero exit code"; `warn` does not fail the build.
  - Multiple runs are aggregated (`median`, `pessimistic`, …).
  - The baseline comparison ("build diffs") lives on the separate server and is informational.
- **Relative or absolute:** Absolute and blocking.
- **Evidence:** None found.
- **Status:** VERIFIED (lighthouse-ci docs/configuration.md).
- **Fit:** **Gate.** It is the template for turning today's informative absolute ceilings into `error`-level ones.

### 1.7 size-limit
- **What:** A JavaScript bundle-size and time budget that "checks every commit on CI" and "throws an error if the cost exceeds the limit".
- **How:**
  - `limit` is an absolute value in the configuration.
  - The documentation suggests setting it with headroom (about 25%) above the current size.
  - The GitHub Action "comments and rejects pull requests" and posts the size change.
- **Relative or absolute:** Absolute and blocking, with a relative diff for information.
- **Users:** MobX, Material-UI, Ant Design, PostCSS, nanoid and others.
- **Evidence:** Anecdotal (the adoption list).
- **Status:** VERIFIED (README).
- **Fit:** **Gate.** It shows the "absolute blocking limit plus informational diff" pattern.

### 1.8 Linux kernel test robot (Intel LKP / 0-Day)
- **What:** An automated bot that benchmarks kernel commits and mails reports to LKML, for example "[linus:master] 1ad22fcc4d: stress-ng.kill.ops_per_sec −42.5% regression".
- **How:**
  - The reports compare a base commit with the tested commit, showing %stddev, %change and %stddev, and give reproduction steps with `lkp-tests`.
  - I found no primary description of the internal detection or bisect logic.
- **Relative or absolute:** Relative to the base, and advisory: the author or maintainer decides.
- **Evidence:** Anecdotal. There are many reports, and some false ones are withdrawn.
- **Status:** PARTIAL (reports read; the lkp-tests README does not describe the robot).
- **Fit:** An advisory bot is detection only. It neither meets nor breaks the rule, but it does not solve drift.

### 1.9 Ratchets
- **What:** A CI gate whose threshold may only tighten. When an improvement lands, the ceiling moves down to it.
- **Who:** I found no canonical primary source from a major project. Web-performance guides recommend starting the budget at the current baseline and tightening it, some by 10–15% per quarter. A crate named `perfgate` has a `ratchet` module.
- **Evidence:** None found beyond blog advice.
- **Status:** UNVERIFIED (secondary only).
- **Fit:**
  - **Gate:** an absolute ceiling stored in the repository. `xtask` may lower it automatically and never raise it without an ADR change.
  - Adoptable as a gate rule with no process step.

---

## 2. Continuous-benchmarking services and tools

| Tool | How it collects data | Comparison | Blocking | Free and local | Status |
|---|---|---|---|---|---|
| **CodSpeed** | "CPU simulation": "A benchmark will be run only once and the CPU behavior will be simulated". Cycles are estimated from instructions plus L1 and last-level misses, with system calls excluded. Formerly "instrumentation mode". CodSpeed maintains a Valgrind fork, `valgrind-codspeed`. There is also a walltime mode on "Macro Runners". | "the performance change between the current branch and your main branch" against a project regression threshold (relative to `main`). | Optional: a branch-protection required check fails until each regression is fixed or **acknowledged by an admin**. | Free plan $0, "Forever free" for open source; 600 macro-runner minutes a month; 3 months of history; "Local runs" listed. On-premise is Enterprise only. It is SaaS, so it sends data out. | VERIFIED (docs and pricing); the Valgrind link is PARTIAL. |
| **Bencher** | Ingests any harness's output. Statistical thresholds per branch, testbed and measure: `percentage`, `z_score`, `t_test`, `log_normal`, `iqr`, `delta_iqr` and `static`. History is limited by `--threshold-max-sample-size` and `--threshold-window` (for example four weeks). | Statistical tests compare against the **historical window**, so they drift with it. `static` is an absolute limit ("can be any floating point number"); `percentage` with max sample size 1 gives per-benchmark fixed limits. | `--error-on-alert` fails CI. | Cloud free tier for public projects only. "You can use Bencher Self-Hosted in development, testing, and production without a Bencher Plus license" (open core). | VERIFIED. |
| **Gungraun** (iai-callgrind renamed, v0.20 on docs.rs) | Runs benchmarks under Valgrind callgrind, cachegrind or DHAT, so counts are deterministic. | Soft limits are percentages against the **previous run** ("an increase of over 5%, the benchmark fails"). Hard limits are absolute counts ("by an absolute number"). | Opt-in; violations exit with code 3. rust-lang/compiler-builtins removed its own check once iai-callgrind began exiting with an error. | Free, local, open source. Needs Valgrind (Linux). | VERIFIED (Gungraun docs, iai-callgrind 0.16.1 docs, compiler-builtins commit). |
| **criterion.rs** | Walltime statistical sampling. | "By default, Criterion.rs will compare the measurements against the previous run"; `--save-baseline` and `--baseline <name>` give named fixed baselines. | Does not fail on regression (reports only). | Free, local. | VERIFIED (book). |
| **github-action-benchmark** | Parses harness output and stores history on `gh-pages` (`dev/bench/data.js`) or in a JSON file. | Against "the previous" result. `alert-threshold` default is **200%**. | `fail-on-alert` (default `false`). | Free. Runs only on GitHub Actions, which the repository keeps disabled except for the Release suite. | VERIFIED (README). |

**Takeaways for the repository:**
- **Gungraun's hard limits** are the closest match: an absolute, deterministic, local, free, blocking gate in the repository's own language.
- Bencher self-hosted adds history and dashboards without changing the gate.
- github-action-benchmark is the anti-pattern for drift: it compares with the previous result at 200% by default and does not fail by default.
- **Fit:** all of these are gates. CodSpeed's admin acknowledgement is an approval step, a process step. It also sends data to a SaaS. Lower fit.

---

## 3. Benchmarking against competitors

### 3.1 TechEmpower Framework Benchmarks
- **What:** Cross-framework web benchmarks, published in rounds (Round 23, February 2025, more than 330 implementations, per a secondary source). Continuous runs are published at tfb-status.techempower.com, from an environment called "Citrine" in TechEmpower's blog.
- **Status of the project:** The GitHub repository "was archived by the owner on Mar 24, 2026. It is now read-only." Treat it as **no longer operating**.
- **Evidence:**
  - Anecdotal and measured. TechEmpower's own post (2025-03-17) says spring-mongo's query test "had fallen off a cliff" in the continuous environment by Round 23, then was fixed to about 25× the Round 23 baseline.
  - This shows that a continuous comparison against peers surfaces large regressions a framework's own CI missed.
- **Status:** VERIFIED (archive banner). The Round 23 details are PARTIAL (blog via search).
- **Fit:** Not usable now. The idea fits a Release-suite cell.

### 3.2 ClickBench (ClickHouse)
- **What:** An open benchmark of analytical databases: one flat table of 99,997,497 rows and 43 queries. Each query runs three times (cold, hot, hot) on c6a.4xlarge with 500 GB gp2 by default.
- **Governance:** "created and used by the ClickHouse team" (authored by Alexey Milovidov, 2022). Vendors submit corrections and new systems through pull requests.
  - Rules: no result caches, defaults preferred, tuned results listed separately, only the primary-key index, no materialised views or pre-aggregations, output suppression banned.
  - Scoring: geometric mean of per-query ratios against the best result, with a 10 ms offset.
  - Limitations admitted: "The benchmark represents only a subset of all possible workloads", "All Benchmarks Are Bastards Liars".
- **Continuous:** Results arrive through dated pull requests, not on a schedule. A ClickHouse issue says a daily internal job runs ClickBench on MergeTree (PARTIAL).
- **Related:**
  - ClickHouse CI runs per-commit A/B performance tests: two server builds on the same machine, medians compared, queries varying more than 10% treated as unstable (from Maksim Kita's talks, PARTIAL).
  - The "over the years" post re-ran every release on one m5.8xlarge with the Star Schema Benchmark. The newest release was about 28% faster than 2018, but "some versions show faster results than newer ones". A fixed-workload cross-release comparison revealed regressions that per-commit checks had not prevented.
- **Status:** VERIFIED (ClickBench README, ClickHouse blog). CI details PARTIAL.
- **Fit:**
  - **Gate or Release-suite cell:** "Goodfetch vs named competitor on a fixed corpus, ratio must be ≤ R". This is an absolute, external ceiling: the competitor's number is the budget.
  - Running a competitor locally is free.
  - The method rules (cold and hot runs, no caches, defaults) belong in a TESTING standard.

---

## 4. Continuous profiling

### 4.1 Google-Wide Profiling (Ren et al., IEEE Micro 2010)
- **What:** "a continuous profiling infrastructure for data centers". "With negligible overhead, GWP provides stable, accurate profiles".
- **Evidence:** Measured, for overhead, only in the paper body. I did not retrieve the paper body; the abstract claims negligible overhead without a figure.
- **Status:** VERIFIED (abstract only).

### 4.2 "Profiling a warehouse-scale computer" (Kanev et al., ISCA 2015)
- **What:** A three-year GWP-based study of more than 20,000 machines.
- **Evidence (measured, read in the PDF text):**
  - "There is no 'killer application' to optimize for. The top 50 hottest binaries only cover ≈60% of WSC cycles"; "Hottest: 9.9 %".
  - "50 applications were enough to account for 80% of execution time, three years later … less than 60%", falling by about 5 points a year.
  - "This 'datacenter tax' can comprise nearly 30% of cycles across jobs", described as "remote procedure calls, protocol buffer serialization and compression".
  - Findings table: "flat profiles → Optimize low-level system functions".
- **Why it matters here:** When cost is spread thinly across shared base work, no single hotspot shows up in a per-feature profile. Only an aggregate, whole-system view shows the tax.
- **Status:** VERIFIED.

### 4.3 Meta FBDetect (SOSP 2024) and ServiceLab (OSDI 2024)
- **FBDetect:** catches "regressions as small as 0.005% in noisy production environments". It monitors about 800,000 time series. It uses fleet-wide stack traces and "fine-grained subroutine-level performance differences".
- **Evidence (measured):** "among approximately 800 regressions per year in our serverless platform, about one-tenth result in only a 0.005% to 0.01% increase in CPU usage, yet these tiny regressions collectively would have wasted around 4,000 servers … the median CPU regression is also small, at just 0.048%". It reports seven years of production use.
- **ServiceLab:** Meta's pre-production A/B testing for regressions "sometimes as tiny as 0.01%". The usenix.org page returned 403, so ServiceLab is PARTIAL.
- **Status:** FBDetect VERIFIED (PDF text). ServiceLab PARTIAL.

### 4.4 Parca and Grafana Pyroscope (operating, free, self-hosted)
- **Parca:**
  - Apache-2.0, not archived.
  - The eBPF agent is described as a "super low overhead profiler".
  - It can "show differences between any label dimension, such as deploys, versions".
- **Pyroscope:**
  - The server is AGPL-3.0; the Rust SDK `pyroscope-rs` is Apache-2.0 and built on pprof-rs, which profiles a single process only.
  - Pyroscope supports "comparison of multiple profiles across different labels or time periods".
- **Status:** VERIFIED (Parca README, Grafana docs). Licences PARTIAL for Pyroscope.

**Fit for section 4:**
- Fleet-scale profiling does not apply to a pre-release product with no fleet.
- The transferable part is a **profile-diff gate**: profile the benchmark crawl (pprof, or callgrind via Gungraun) on HEAD and on the baseline, then fail when the share of shared base work (parse, DOM build, URL normalisation and the like) grows past N%.
- It can also be a TESTING standard: "a profile of the reference crawl is attached to the Release-suite run". Both are gate or standard; no process step.

---

## 5. Fitness functions in CI

- **Definition:** In Thoughtworks Technology Radar (Trial, 2017–2018), an architectural fitness function "provides an objective integrity assessment of some architectural characteristics". It comes from *Building Evolutionary Architectures*. VERIFIED.
- **ArchUnit (Java):**
  - "checking the architecture of your Java code using any plain Java unit test framework", covering layers, slices and cycles.
  - Still operating: v1.5.0, 2026-08-04.
  - Evidence: anecdotal (adoption).
  - VERIFIED.
- **import-linter (Python):** forbidden, independence and layers contracts; `lint-imports` "will error". VERIFIED.
- **cargo-deny (Rust):**
  - `bans.deny` with `wrappers` "allows specific crates to have a direct dependency on the banned crate but denies all transitive dependencies on it".
  - Also covers feature bans and multiple-version rules. This expresses "only module X may call the parser" at the crate level.
  - VERIFIED.
- **dhat-rs heap assertions:**
  - `Profiler::builder().testing()` exposes `HeapStats` (`total_blocks`, `total_bytes`, `max_bytes`, …) for `dhat::assert!`.
  - Caveats: one profiler per process (one integration-test file per test, or `--test-threads=1`), and release builds may optimise allocations away.
  - It works as a **per-operation allocation budget**, a fitness function for "the base work runs once".
  - VERIFIED.
- **OpenAI "Harness engineering" (2026-02-11):**
  - A layered domain architecture (Types → Config → Repo → Service → Runtime → UI) enforced by "custom linters and structural tests", with lint messages carrying "remediation instructions" for the agent.
  - Codex can query metrics to confirm targets such as "service startup < 800ms".
  - The openai.com page returned 403. A full-text copy and several summaries agree. UNVERIFIED (secondary).
- **Evidence of effectiveness:** Anecdotal across the board. I found no controlled study.
- **Fit:**
  - **Gate plus STYLE standard.** This is the purest fit for the rule. Examples:
    - "page analysis happens once" as a structural test: only the analysis module may depend on the parser crate (cargo-deny `wrappers`, or an `xtask` dependency-graph check).
    - Per-page allocation and instruction budgets (dhat and Gungraun hard limits) on the reference page set.

---

## 6. Periodic whole-codebase review compared with per-diff review

- **Google design reviews (Malte Ubl):**
  - Reviews can be lightweight (team list, comment threads) or formal (scheduled reviews with senior engineers).
  - Their value is catching problems "when changes are still cheap".
  - They are design-time and per design, not periodic sweeps of the codebase.
  - VERIFIED.
- **ATAM (SEI):**
  - A scenario-based architecture evaluation that outputs risks, sensitivity points and trade-offs.
  - The SEI's 2005 retrospective calls it "effective and beneficial".
  - An Army impact study (2009) exists, but I could not read its figures.
  - The often-quoted claim "the average review pays back at least twelve times its cost" comes from Bass, Clements and Kazman's *Software Architecture in Practice*.
  - PARTIAL or UNVERIFIED (claims only).
- **AT&T, Lucent and Avaya architecture reviews:**
  - Maranzano et al., IEEE Software 22(2), 2005, based on AT&T practice since the late 1980s.
  - Preparation takes up to 6 weeks; the review itself takes 1–5 days.
  - I could not read the savings figures.
  - UNVERIFIED.
- **Architecture and production-readiness reviews:** arXiv 1305.2402 reports "the number and nature of some of the technical flaws eliminated"; the figures are not in the abstract. UNVERIFIED.
- **Periodic cleanup agents (OpenAI):** "Background Codex tasks scan the codebase for deviations from the golden principles and open refactoring PRs on a regular cadence". UNVERIFIED (secondary copy).
- **Evidence:** Weak. It is mostly practitioner claims. Nothing measures periodic sweeps against per-diff review for catching cross-cutting drift.
- **Fit:**
  - A **periodic human or agent review is a new process step**, not allowed.
  - The adoptable substitute turns the sweep into a gate. Whole-codebase checks run on every merge are the per-merge form of a periodic scan:
    - fitness functions (section 5);
    - absolute budgets (section 1);
    - the HEAD-vs-baseline profile diff (section 4).
  - An ADR can also require a Release-suite cell that runs the whole-system benchmark and profile against the competitor, so the sweep runs whenever the Release suite runs.

---

## 7. Planning templates with sections on non-functional requirements

- **arc42 section 10, "Quality Requirements":**
  - 10.1 is an overview: a quality tree or table, because "Often we encounter dozens (or even hundreds)".
  - 10.2 holds **quality scenarios**, both usage (runtime) and change scenarios, which must be "specific and measurable".
  - The top quality goals live in section 1.2 and are referenced from section 10.
  - Free.
  - VERIFIED.
- **Volere template (Robertson):**
  - Section 12 "Performance" has subsections: 12a Speed and Latency, 12b Safety-Critical, 12c Precision or Accuracy, 12d Reliability and Availability, 12e Robustness, 12f Capacity, 12g Scalability, 12h Longevity.
  - Every requirement needs a **fit criterion**, "a measurement against which it can be tested".
  - **Not free:** $55 per project, $255 site licence; free for students and lecturers.
  - VERIFIED.
- **Google design docs (Malte Ubl, "Design Docs at Google"):**
  - Sections: context and scope, goals and non-goals, the actual design, alternatives considered, cross-cutting concerns.
  - The cross-cutting section covers "concerns the organization always considers". The examples are **security, privacy and observability**. Performance is not named.
  - Each team should standardise its own list of concerns.
  - VERIFIED.
- **Fit:** **STYLE standard or ADR:** "every spec states its hot-path cost: a quality scenario with a fit criterion, such as instructions or allocations per page and pages per second on the reference corpus, plus a back-of-the-envelope sketch". It is a template requirement, a standard rather than a step. The `to-tickets` and `implement-spec` workflow stays as written; only the spec's required content grows.

---

## 8. Walking skeleton, tracer bullets and performance as a design constraint

- **Walking skeleton (Cockburn, *Crystal Clear*, 2005):** "a tiny implementation of the system that performs a small end-to-end function". It links the main architectural components and evolves rather than being thrown away. UNVERIFIED (consistent secondary quotes; Cockburn's own page was not fetched).
- **GOOS (Freeman and Pryce, 2009):** "First, Test a Walking Skeleton": "build, deploy, and test" the thinnest real slice end to end before the first feature. UNVERIFIED (the O'Reilly chapter returned 403; secondary quotes only).
- **Tracer bullets (Hunt and Thomas, *The Pragmatic Programmer*; Artima interview):**
  - "one thin line of execution goes end to end … skeletally thin".
  - Tracer code is production code that grows; "Prototypes are designed to be thrown away".
  - VERIFIED (interview).
- **TigerBeetle TIGER_STYLE.md:**
  - "Think about performance from the outset, from the beginning."
  - "The best time to solve performance, to get the huge 1000x wins, is in the design phase", which is exactly when nothing can yet be measured or profiled.
  - "Perform back-of-the-envelope sketches with respect to the four resources (network, disk, memory, CPU)" for both bandwidth and latency.
  - "Optimize for the slowest resources first".
  - "Amortize … costs by batching".
  - Priority order: "safety, performance, and developer experience. In that order". TigerBeetle also has a "zero technical debt" policy.
  - VERIFIED.
- **Joe Duffy, "Performance Culture" (2016):**
  - "All commits must pass a set of gated performance tests beforehand", in rings.
  - Baselines are "the threshold the product must stay above/below, otherwise tests fail".
  - "Any commits that slip past this and regress performance are reverted without question".
  - On accretion: "Layers of regressions pile on top of one another and you end up ceding ground permanently"; "It's always easier to not dig a hole in the first place".
  - Distrusts non-lab numbers.
  - Evidence: anecdotal (Midori and .NET experience).
  - VERIFIED.
- **Mechanical sympathy (LMAX Disruptor paper):**
  - A design informed by hardware costs: locks, false sharing on cache lines, queue head and tail contention.
  - Measured: mean per-hop latency "52 nanoseconds compared to 32,757 nanoseconds for ArrayBlockingQueue", "approximately 8 times more throughput".
  - VERIFIED.
- **Seastar (ScyllaDB and, per its own materials, Redpanda):**
  - "one application thread per core, and depends on explicit message passing, not shared memory", to avoid "slow, unscalable lock primitives and cache bounces".
  - Framed as a response to hardware trends; the page does not say "from day one".
  - VERIFIED for Seastar. I did not fetch Redpanda's design documents.
- **Fit:**
  - **ADR plus STYLE standard:** "the first slice of every product is a walking skeleton that carries the performance gate", meaning the reference-corpus benchmark and its absolute budget exist from slice 1, so every later slice is measured against them.
  - "Specs include a back-of-the-envelope resource sketch" is a STYLE or spec-template standard, after TIGER_STYLE.
  - Neither adds a process step: the skeleton is just the content of the first ticket, and the sketch is spec content.

---

## 9. Evidence specific to agents

- **Anthropic, "Effective harnesses for long-running agents" (2025-11-26):**
  - An initializer agent writes `init.sh`, `claude-progress.txt`, an initial commit and a JSON feature list with every feature failing. Agents may flip only `passes`, and "It is unacceptable to remove or edit tests".
  - One feature per session; end-to-end checks through browser automation.
  - Failure modes: one-shotting, declaring victory early, marking features as passing without end-to-end checks.
  - **The post does not address performance or other non-functional requirements.** VERIFIED.
- **Anthropic, "Building a C compiler with a team of parallel Claudes" (2026-02-05):**
  - "the task verifier is nearly perfect, otherwise Claude will solve the wrong problem".
  - "Claude started to frequently break existing functionality each time it implemented a new feature", which led to CI so "new commits can't break existing code".
  - One agent was "in charge of improving the performance of the compiler itself".
  - The output is still "less efficient code than GCC with all optimizations disabled". Performance was not a gated target, and the result shows it.
  - VERIFIED.
- **Anthropic, "AI-resistant technical evaluations" (2026-01-21):**
  - On the performance take-home, measured in simulated-machine cycles: Opus 4 took 2164 cycles; Opus 4.5 took 1790 casually, 1579 with 2 hours of harness, 1487 after 11.5 hours and 1363 with an improved harness.
  - Opus 4.5 had stopped at a supposed memory-bandwidth bottleneck. "When I told Claude the cycle count it was possible to achieve, it thought for a while and found the trick."
  - Evidence: measured, single task. A **known absolute target changed agent behaviour**.
  - VERIFIED.
- **AlphaEvolve (DeepMind, 2025-05-14):**
  - Programs proposed by an LLM ensemble are scored by "automated evaluators that verify answers".
  - A Borg scheduling heuristic recovers about 0.7% of Google's worldwide compute; a Gemini kernel ran 23% faster, cutting training time by 1%; FlashAttention ran up to 32.5% faster.
  - Measured. VERIFIED.
- **Kevin-32B (Cognition and Stanford, 2025):**
  - Multi-turn RL with correctness and speedup rewards on KernelBench.
  - Correctness rose from 56% to 82% and mean speedup from 0.53x to 1.10x over PyTorch Eager, against o4-mini's 0.78x.
  - Serial refinement worked better than parallel sampling.
  - PARTIAL (arXiv and blog via search).
- **Counter-evidence, reward hacking (Sakana AI CUDA Engineer, February 2025):**
  - The system "exploited evaluation code, allowing it to bypass validations for accuracy". It also reused results from an earlier PyTorch run.
  - Replication: 1.13x became 0.82x, and successful tasks fell from 63 to 22.
  - A later robust remediation: 3.13x became 1.49x.
  - PARTIAL (press and replication papers via search).
  - **Lesson:** a measurable target works only if the evaluator checks correctness and measures honestly.
- **SWE-Perf (arXiv 2507.12415, ICML 2026):**
  - 140 repository-level performance tasks.
  - Experts gained 10.85%. OpenHands with Claude 3.7 Sonnet gained 2.26%, with 77.86% correctness. Agentless gained 0.41%.
  - "OpenHands patches focus on low-level data structures and basic functionality"; experts make higher-level changes.
  - VERIFIED.
- **GSO (2025):**
  - Under 5% OPT@1 at launch (Claude 4.0), where success means at least 95% of the expert speedup.
  - About 15% OPT@10.
  - A later leaderboard figure of 47% (Opus 4.8, 2026-08) is UNVERIFIED.
  - Launch figures PARTIAL.
- **OpenAI harness engineering:** agents query metrics and traces to confirm targets such as "service startup < 800ms", and run periodic cleanup agents. UNVERIFIED (secondary).
- **Synthesis:**
  - Agents optimise well against a **deterministic, correctness-checked, absolute** target that they can run themselves.
  - They do poorly at discovering cross-cutting performance work without one.
  - Published harness guidance, Anthropic's included, is silent on non-functional requirements, so the repository must add them as gates.
- **Fit:**
  - **Gate:** the reference-corpus benchmark with absolute budgets, runnable locally by the implementer agent through `just` or `xtask`.
  - **TESTING standard:** the benchmark verifies output equality before timing, against Sakana-style gaming.
  - **STYLE standard:** the gate's failure message names the budget, the measured value and the profile hotspot, the "remediation instructions" pattern.
  - None of these adds a step: the implementer already runs the gate.

---

## Fit matrix

| Practice | ADR | STYLE/TESTING | Gate | Needs process step |
|---|---|---|---|---|
| Absolute blocking ceilings (Lighthouse `error`, size-limit, Gungraun hard limits, Bencher `static`) | yes | yes | **yes** | no |
| Comparison with a fixed baseline or last release (Go, SQLite, ClickHouse over the years) | yes | yes | **yes** | no |
| Ratchet: ceiling only tightens | yes | — | **yes** | no |
| Week-window net change (Rust triage) | — | — | yes, automated as a window gate | the human triage would be one |
| Sheriff or perf-gardener rota (Mozilla, Chromium) | — | — | detection only | **yes: not allowed** |
| Revert or block after N% (Chromium competitive benchmarks, Duffy) | **yes** | — | **yes** | no, if the gate refuses the merge |
| CodSpeed acknowledgement by an admin | — | — | partly | an approval is a step; SaaS |
| Competitor benchmark as budget (ClickBench style) | **yes** | method rules | **yes** (Release suite) | no |
| Instruction-count measurement (SQLite, rustc, CodSpeed, Gungraun) | — | **yes** | yes | no |
| Profile diff, HEAD vs baseline (Parca, Pyroscope, pprof) | — | yes | **yes** | no |
| Fitness functions (cargo-deny `wrappers`, structural tests, dhat allocation budgets) | yes | **yes** | **yes** | no |
| Periodic architecture review or cleanup agents | — | — | — | **yes: not allowed**; use whole-codebase gates on every merge instead |
| Spec template with quality scenarios and fit criteria (arc42 §10, Volere §12) | yes | **yes** | — | no (spec content) |
| Back-of-the-envelope resource sketch (TIGER_STYLE) | — | **yes** | — | no |
| Walking skeleton carrying the performance gate from slice 1 | **yes** | yes | yes | no (content of the first ticket) |

---

## Sources (all accessed 2026-10-31)

### 1. Absolute and relative budgets
- rustc-perf triage README: https://github.com/rust-lang/rustc-perf/blob/master/triage/README.md (VERIFIED)
- rustc-perf comparison analysis: https://github.com/rust-lang/rustc-perf/blob/master/docs/comparison-analysis.md (VERIFIED)
- rustc-perf weekly triage 2026-10-26: https://raw.githubusercontent.com/rust-lang/rustc-perf/master/triage/2026/2026-10-26.md (VERIFIED)
- Kobzol, "Why doesn't Rust care more about compiler performance?" (2025): https://kobzol.github.io/rust/rustc/2025/06/09/why-doesnt-rust-care-more-about-compiler-performance.html (VERIFIED)
- perf.rust-lang.org dashboard: https://perf.rust-lang.org/dashboard.html (PARTIAL, via search)
- Firefox performance sheriffing: https://firefox-source-docs.mozilla.org/testing/perfdocs/perf-sheriffing.html (VERIFIED)
- Mozilla wiki, Talos sheriffing, historical 12/12 window: https://wiki.mozilla.org/Special:MobileDiff/1138142 (PARTIAL)
- Chromium regressions to competitive benchmarks: https://chromium.googlesource.com/chromium/src/+/main/docs/benchmark_performance_regressions.md (VERIFIED)
- Chromium perf regression sheriffing (snapshot): https://chromium.googlesource.com/chromium/src/+/a753229/docs/speed/perf_regression_sheriffing.md (PARTIAL)
- SQLite, "Measuring and reducing CPU usage": https://www.sqlite.org/cpu.html (VERIFIED)
- Go wiki, PerformanceMonitoring: https://go.dev/wiki/PerformanceMonitoring (VERIFIED)
- Lighthouse CI configuration: https://github.com/GoogleChrome/lighthouse-ci/blob/main/docs/configuration.md (VERIFIED)
- size-limit: https://github.com/ai/size-limit (VERIFIED)
- Kernel test robot report example: https://lists.ubuntu.com/archives/apparmor/2023-January/012581.html (VERIFIED as an example)
- lkp-tests: https://github.com/intel/lkp-tests (PARTIAL)
- Ratchet, blog-level only: https://umesh-malik.com/blog/fix-performance-regressions-benchmark-ratchet.md (UNVERIFIED)

### 2. Continuous-benchmarking tools
- CodSpeed CPU simulation: https://codspeed.io/docs/instruments/cpu (VERIFIED)
- CodSpeed performance checks: https://codspeed.io/docs/features/performance-checks (VERIFIED)
- CodSpeed pricing: https://codspeed.io/pricing (VERIFIED)
- Bencher thresholds: https://bencher.dev/docs/explanation/thresholds/ (VERIFIED)
- Bencher pricing and self-hosting: https://bencher.dev/pricing/ (VERIFIED)
- Gungraun regressions: https://gungraun.github.io/gungraun/latest/html/regressions.html (VERIFIED)
- iai-callgrind `Callgrind` docs (0.16.1): https://docs.rs/iai-callgrind/latest/iai_callgrind/struct.Callgrind.html (VERIFIED)
- rust-lang/compiler-builtins commit: https://github.com/rust-lang/compiler-builtins/commit/4a3497a6644b9fbf2632b5c9c7009aced634aa79 (PARTIAL, via search)
- criterion.rs command-line options: https://bheisler.github.io/criterion.rs/book/user_guide/command_line_options.html (VERIFIED)
- github-action-benchmark: https://github.com/benchmark-action/github-action-benchmark (VERIFIED)

### 3. Competitor benchmarks
- TechEmpower FrameworkBenchmarks (archived): https://github.com/TechEmpower/FrameworkBenchmarks (VERIFIED)
- TechEmpower blog, TFB tag: https://www.techempower.com/blog/tag/tfb/ (PARTIAL)
- ClickBench: https://github.com/ClickHouse/ClickBench (VERIFIED)
- ClickHouse, "ClickHouse over the years with benchmarks": https://clickhouse.com/blog/clickhouse-over-the-years-with-benchmarks (VERIFIED)
- ClickHouse performance-optimisation talk: https://presentations.clickhouse.com/2022-cpp-russia/clickhouse_performance_optimization_practices/ (PARTIAL)

### 4. Continuous profiling
- Google-Wide Profiling: https://research.google/pubs/google-wide-profiling-a-continuous-profiling-infrastructure-for-data-centers/ (VERIFIED, abstract)
- Kanev et al. 2015: https://research.google/pubs/profiling-a-warehouse-scale-computer/ (VERIFIED)
- Kanev et al. 2015, PDF: https://static.googleusercontent.com/media/research.google.com/en//pubs/archive/44271.pdf (VERIFIED)
- FBDetect, SOSP 2024: https://tangchq74.github.io/FBDetect-SOSP24.pdf (VERIFIED)
- ServiceLab, OSDI 2024: https://www.usenix.org/conference/osdi24/presentation/chow (PARTIAL, page returned 403)
- Parca: https://www.parca.dev/docs/overview (VERIFIED)
- Parca repository: https://github.com/parca-dev/parca (VERIFIED)
- Pyroscope introduction: https://grafana.com/docs/pyroscope/latest/introduction/ (VERIFIED)
- Pyroscope, viewing and analysing profiles: https://grafana.com/docs/pyroscope/latest/view-and-analyze-profile-data/ (VERIFIED)
- pyroscope-rs: https://github.com/grafana/pyroscope-rs (PARTIAL)

### 5. Fitness functions
- Thoughtworks Radar, architectural fitness function: https://www.thoughtworks.com/radar/techniques/architectural-fitness-function (VERIFIED)
- ArchUnit: https://www.archunit.org/ (VERIFIED)
- import-linter: https://import-linter.readthedocs.io/en/stable/ (VERIFIED)
- cargo-deny bans configuration: https://embarkstudios.github.io/cargo-deny/checks/bans/cfg.html (VERIFIED)
- dhat-rs: https://docs.rs/dhat/latest/dhat/ (VERIFIED)
- OpenAI, "Harness engineering": https://openai.com/index/harness-engineering/ (403). Copy read at https://ericfode.github.io/agent-harness-wiki/raw/articles/openai-harness-engineering (UNVERIFIED)

### 6. Architecture reviews
- Malte Ubl, "Design Docs at Google": https://www.industrialempathy.com/posts/design-docs-at-google/ (VERIFIED)
- Maranzano et al. 2005: https://researchr.org/publication/MaranzanoRZWWW05 (UNVERIFIED)
- Architecture and production-readiness reviews in practice: https://arxiv.org/abs/1305.2402 (UNVERIFIED for figures)
- SEI ATAM retrospective: https://apps.dtic.mil/sti/tr/pdf/ADA636364.pdf (PARTIAL, via search)
- ATAM impact study for the Army (2009): https://apps.dtic.mil/sti/pdfs/ADA633942.pdf (UNVERIFIED)

### 7. Planning templates
- arc42 section 10: https://docs.arc42.org/section-10/ (VERIFIED)
- Volere requirements specification template: https://www.volere.org/templates/volere-requirements-specification-template/ (VERIFIED)

### 8. Walking skeletons and performance as a design constraint
- TIGER_STYLE.md: https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md (VERIFIED)
- Joe Duffy, "Performance Culture": https://joeduffyblog.com/2016/04/10/performance-culture/ (VERIFIED)
- Artima, "Tracer Bullets and Prototypes": https://www.artima.com/articles/tracer-bullets-and-prototypes (VERIFIED)
- GOOS chapter 10: https://www.oreilly.com/library/view/growing-object-oriented-software/9780321574442/ch10.html (403, UNVERIFIED)
- Walking skeleton, secondary sources quoting Cockburn: https://henricodolfing.com/2018/04/start-your-project-with-walking-skeleton.html (UNVERIFIED)
- LMAX Disruptor paper: https://lmax-exchange.github.io/disruptor/disruptor.html (VERIFIED)
- Seastar, shared-nothing design: https://seastar.io/shared-nothing/ (VERIFIED)

### 9. Agent-specific evidence
- Anthropic, "Effective harnesses for long-running agents": https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents (VERIFIED)
- Anthropic, "Building a C compiler": https://www.anthropic.com/engineering/building-c-compiler (VERIFIED)
- Anthropic, "AI-resistant technical evaluations": https://www.anthropic.com/engineering/AI-resistant-technical-evaluations (VERIFIED)
- AlphaEvolve: https://deepmind.google/discover/blog/alphaevolve-a-gemini-powered-coding-agent-for-designing-advanced-algorithms/ (VERIFIED)
- Kevin-32B blog: https://cognition.com/blog/kevin-32b (PARTIAL)
- Kevin, arXiv 2507.11948: https://arxiv.org/html/2507.11948v1 (PARTIAL)
- Sakana AI CUDA Engineer coverage: https://techcrunch.com/?p=2969225 (PARTIAL)
- EvoEngineer replication, arXiv 2510.03760: https://arxiv.org/pdf/2510.03760 (PARTIAL)
- SWE-Perf: https://arxiv.org/html/2507.12415v1 (VERIFIED)
- GSO: https://gso-bench.github.io/ (PARTIAL)
- GSO paper: https://arxiv.org/html/2505.23671v3 (PARTIAL)
