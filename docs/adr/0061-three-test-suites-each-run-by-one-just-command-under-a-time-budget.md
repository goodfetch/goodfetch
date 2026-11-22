# Three test suites, each run by one `just` command under a time budget, and the release suite gates every merge

Goodfetch's tests run in three suites, each a superset of the one before: the **Quick suite**, the **Pre-merge suite** and the **Release suite**. Each has one `just` command and a time budget. The Release suite holds the full platform, runtime and browser matrix of ADR 0024, and it runs before every merge, in the merge queue, as well as on `main` every night and for every release.

The owner asked for "tiered suites by speed: quick (every change), pre-merge, and release (full matrix), each run by one local command, each with a time budget". ADR 0032 runs the full matrix in the merge queue, so read literally the two conflict: the owner's words put the full matrix in "release", and ADR 0032 runs it before every merge. That is conflict 4 on the test strategy map. The evidence says to keep the matrix before the merge, and to call it the Release suite:

- **Peers that ship on many platforms gate merges on them.** Rust's pull-request builds are five Linux jobs of about 40 minutes, and its merge builds run "tens of CI jobs ... across operating systems and targets", taking about 2 hours ([rustc-dev-guide](https://rustc-dev-guide.rust-lang.org/tests/ci.html)). Cargo's merge queue runs macOS arm64 and Windows x64 and arm64 test jobs of 14 to 21 minutes ([findings](https://github.com/goodfetch/goodfetch/blob/research/rust-test-tooling/rnd/research/2026-10-13-rust-test-tooling.md), §12.3).
- **A failure found after the merge costs more.** It lands on `main`, which carries the nightly channel and from which release branches are cut (ADR 0032), and someone must bisect it.
- **Other decisions need the matrix before the merge.** ADR 0057's increase check compares each target's build with `main`'s, and ADR 0032 lets a pull request run only the Sinks it changes because the merge queue then runs them all.
- **The merge queue doesn't hold the author up.** Once queued, the pull request merges or comes back without anyone waiting on it. The free plan's 20 concurrent jobs, 5 of them macOS, are why the matrix runs there and not on every pull request ([GitHub Actions limits](https://docs.github.com/en/actions/reference/limits)).
- **Moving the matrix to release time only** would put macOS, Windows, arm64 and every runtime past the merge. Their failures would surface at the nightly or the release candidate, after other changes had built on them.

Decided in [Test suites, local commands and time budgets](https://github.com/goodfetch/goodfetch/issues/276), with the agent answering on the owner's delegation. It rests on [Rust test tooling in 2026](https://github.com/goodfetch/goodfetch/issues/263), [Performance regression testing in CI](https://github.com/goodfetch/goodfetch/issues/272), [Flaky tests without retries](https://github.com/goodfetch/goodfetch/issues/270) and [Upgrade and version-mismatch testing](https://github.com/goodfetch/goodfetch/issues/271). The rules every test follows are in `TESTING.md`.

## Decision

### The three suites

Each suite runs everything the suite before it runs.

- **Quick suite**: formatting and lint checks, and the in-process tests, of what a change touches.
  - Rust: unit tests in `src/` (nextest's `kind(lib)`, `kind(bin)` and `kind(proc-macro)`), never `tests/it`.
  - TypeScript and Python: each runner's in-process selection, which [Test layout, naming, shared helpers and the test-first process](https://github.com/goodfetch/goodfetch/issues/277) names.
  - The committed format fixtures, one per format version (the upgrade research's fit; [Upgrade and version-mismatch testing](https://github.com/goodfetch/goodfetch/issues/287) decides their shape).
  - `xtask codegen --check` and `xtask affected`'s own tests.
  - Nothing that starts a process, a browser or a container, or binds a socket.
  > Amended by [ADR 0097](0097-one-docs-site-renders-each-products-section-from-one-source-and-neither-product-leads-with-access.md): the Quick suite also runs `xtask wording`, which fails on a term of ADR 0035's wording list in published prose.
  > Amended by [ADR 0126](0126-each-crate-declares-which-of-its-modules-may-import-which-each-module-names-one-concern-and-a-file-past-1000-lines-states-why.md): the Quick suite also runs the `xtask` check of each crate's `modules.toml` and of the 1,000-line size alarm.
- **Pre-merge suite**: what ADR 0032's pull-request tier runs, on one platform. That is the Quick suite plus:
  - the `tests/it` binary of every touched crate and of the crates that depend on it (`rdeps`);
    > Amended on 2026-11-09 (owner request of 2026-11-09, [#2238](https://github.com/goodfetch/goodfetch/issues/2238)): a change confined to a crate's `tests/` folder touches that crate's tests alone, since no library, executable or benchmark build reads that folder: it reaches neither the crates that depend on the crate, nor what runs its executable, nor its bench targets. A file of that folder that a file outside it names (a shared fixture listing it, the crate's own code including it, another crate naming it through the crate's folder) still touches the whole crate.
  - the conformance suites against the local Proving ground with the Goodfetch Chromium;
  - the touched Sinks' suites against their emulators and containers (every Sink's suite until the merge queue exists, as ADR 0032 requires);
  - the touched SDKs, adapters and Integrations on one runtime each;
  - the Console's component and end-to-end tests on Chromium;
    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no suite, test layout, Journey surface, security boundary, Performance budget, Feature kind (`screen:`) or actor exists for the Console.
  - the docs build and examples when docs change;
  - the static checks ADR 0032 lists.
- **Release suite**: everything the Pre-merge suite runs, for every package rather than only the touched ones, across the full matrix of ADR 0024:
  - every platform: Linux x64 and arm64 on glibc and musl, macOS arm64 and Intel, Windows x64 and arm64;
    > Amended by [ADR 0073](0073-accessibility-meets-wcag-2-2-aa-the-console-ships-ready-to-translate-and-screenshots-compare-in-one-pinned-image.md): visual tests run on one platform, Linux x86-64 inside the pinned Playwright image.
  - every runtime: every supported Node line, Bun, Deno 2, and every supported CPython, with its free-threaded build on Linux (on every OS in the nightly, ADR 0032);
    > Amended by [ADR 0068](0068-the-console-is-tested-in-real-browsers-and-served-by-the-real-binary.md): the Console's and the docs site's tests run on one Node line; their matrix is browsers. The docs site's build, lychee and Playwright pass run on Linux only.
    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console, so what this says of the Console ends; the rest stands.
  - browser conformance on every OS;
  - every Sink's suite;
  - the deployment-target smoke runs that CI hosts (ADR 0049);
  - the tests that need released binaries, such as version skew and rolling upgrades (decided by [Upgrade and version-mismatch testing](https://github.com/goodfetch/goodfetch/issues/287)).
  > Amended by [ADR 0094](0094-both-products-are-released-and-signed-together-and-each-test-runs-on-collie-unless-it-needs-foxhound.md): each suite covers both Products. A test that runs an executable runs Collie's when Collie carries everything it needs and Foxhound's otherwise, `xtask affected` reaches only Foxhound's tests from a change confined to Foxhound's side, and each target's job builds both executables.

Checks that are non-deterministic by nature are not in any suite: real SaaS Sinks, live models, the online Proving ground, real vendors, Chrome's beta and dev channels, and the benchmark. They keep the tiers ADRs 0026, 0032 and 0043 give them. How they report, and when they block, is decided by [Flakiness policy and checks that are non-deterministic by nature](https://github.com/goodfetch/goodfetch/issues/280).
> Amended by [ADR 0074](0074-the-goodfetch-chromium-is-gated-where-it-is-built-against-a-control-build-of-the-same-revision.md): the Chromium build gate's pinned tests are in no suite too, since they need the build's source tree.

### Tiers run suites

| CI tier (ADR 0032) | What it runs |
|---|---|
| Pull request | The Quick suite over the whole workspace (`just quick all`) first, as its own job, for the first signal. Then the rest of the Pre-merge suite on Linux, selected by `xtask affected`. "all green" is the only required check. |
| Merge queue (`merge_group`) | The Release suite, plus ADR 0057's merge-queue size checks: the increase check per target, the crate's ceiling, and the OCI images built for their own increase check. "all green" there is required too. |
| `main` after a merge | No tests: the merge queue already ran the Release suite on the same tree. A cache-warming job saves the caches (below). |
| Nightly | The Release suite again on `main`'s head, then the nightly-only checks and the nightly publish. A second run of the same commit is what result history needs (the flaky-tests findings). |
| Release candidate, stable release, patch | The Release suite on the release branch's commit, then that release's gates (ADR 0032). |

> Amended by [ADR 0074](0074-the-goodfetch-chromium-is-gated-where-it-is-built-against-a-control-build-of-the-same-revision.md): the Chromium build gate is also a required check, on every head.
> Amended by [ADR 0135](0135-while-the-repository-is-private-the-owners-machine-runs-the-pull-request-tier-in-a-throwaway-linux-vm-and-private-fuzzing-nightly.md): while the repository is private, every job of the pull-request tier runs on the owner's machine, its build and tests in a Linux VM recreated for the job with an x86-64 userland under Rosetta for Linux, never on GitHub-hosted runners.

- **Until the merge queue exists** (the repository is private, ADR 0032), the Release suite runs on manual dispatch. It must pass on the commit before each milestone's owner test (ADR 0058), and before any release.
  > Note (2026-11-22): the repository has been public since that day ([ADR 0135](0135-while-the-repository-is-private-the-owners-machine-runs-the-pull-request-tier-in-a-throwaway-linux-vm-and-private-fuzzing-nightly.md)'s note of 2026-11-22), but the merge queue does not exist yet, so the Release suite still runs on manual dispatch; the hosted-CI spec (spec to be linked) tracks the queue.
  > Amended by [ADR 0108](0108-while-the-repository-is-private-the-release-suite-runs-its-macos-cells-on-the-owners-machine-and-its-hosted-cells-under-a-ceiling.md): while the repository is private, a manual-dispatch run puts its macOS cells on the owner's machine (macOS x64 under Rosetta 2), runs Linux x64, Linux arm64 and Windows x64 hosted under a ceiling of 600 included minutes a month, and leaves native macOS Intel and Windows arm64 until the repository is public.
- **Size baseline.** Every merge-queue run saves its per-target size report (`xtask size`, thin-LTO CI profile) keyed by the tree it tested. The group that merges makes that tree `main`'s head, so its report is `main`'s build of the same profile that ADR 0057's increase checks and the pull request's Linux x64 estimate compare against; they read the report for `main`'s head. While the repository is private, manual-dispatch Release runs on `main` save it.
- **Merge queue settings:** "Only merge non-failing pull requests" on, and a status check timeout of 120 minutes. Group size and build concurrency are set so the queue keeps up with merges, within the free plan's 5 concurrent macOS jobs.

### One `just` command per suite

These are the recipes that [Workspace skeleton and the "all green" pull-request check](https://github.com/goodfetch/goodfetch/issues/132) creates. CI calls the same recipes.

| Recipe | What it does |
|---|---|
| `just quick` | The Quick suite over what changed since the merge base with `origin/main`, uncommitted changes included. |
| `just quick all` | The Quick suite over the whole workspace. |
| `just pre-merge` | The Pre-merge suite over what changed, on the host OS. |
| `just pre-merge all` | The Pre-merge suite over everything, on the host OS. |
| `just release` | The host OS's cell of the Release suite, with every runtime installed on the host. |
| `just release-ci` | Dispatches the Release suite's workflow on the current branch's pushed head (`gh workflow run`) and follows it to the end (`gh run watch`). Its exit code is the run's verdict. |
| `just fmt` | Formats Rust, TypeScript and Python in place. |
| `just lint` | Checks formatting, then runs the linters and type checkers `STYLE.md` names: Clippy, oxlint, ruff, pyright, mypy's API check, `typos`, zizmor, `cargo-deny` and `cargo-shear`. |
| `just build` | Builds the workspace. |
| `just test <suite> [packages]` | Runs one suite's tests alone, without lints, for Rust, TypeScript or Python packages. CI jobs call this for their part. |
| `just hooks` | Opt-in: points `core.hooksPath` at the committed `.githooks/`. Its pre-commit hook checks formatting, and its pre-push hook runs `just quick`. |

> Amended by [ADR 0108](0108-while-the-repository-is-private-the-release-suite-runs-its-macos-cells-on-the-owners-machine-and-its-hosted-cells-under-a-ceiling.md): while the repository is private, `just release-ci` also takes the cells to run, all by default, and on `main` a commit of `main`, its head by default; off `main` it tests the branch's pushed head, hosted cells only. A commit's verdict may then span several dispatches.

- **Until `xtask affected` exists** ([`xtask affected`: change detection for the pull-request tier](https://github.com/goodfetch/goodfetch/issues/134), which comes after the skeleton), `just quick` and `just pre-merge` run over the whole workspace, as their `all` forms do.
- **`xtask affected` is the one source of what changed**, in CI and locally. Locally it diffs the working tree against the merge base. It gives nextest an `rdeps(…)` filterset, so crates that depend on a changed crate are tested too, and gives Vitest and pytest their packages.
- **A local suite that can't run a part on this host says so.** For example, the sink containers on a machine without a container engine, or another OS's cell. The recipe lists every part it skipped, with the reason, after its results. Its exit code covers what ran. CI is the verdict.
- **The reference host is never reached from a recipe.** `just release-ci` runs on GitHub-hosted runners only. The benchmark keeps ADR 0025's own command and ADR 0032's trigger rule: tags and manual dispatch by the owner.
  > Amended by [ADR 0108](0108-while-the-repository-is-private-the-release-suite-runs-its-macos-cells-on-the-owners-machine-and-its-hosted-cells-under-a-ceiling.md): while the repository is private, `just release-ci` on `main` also runs the macOS cells on the owner's machine, testing only commits of `main`; on another branch it runs the hosted cells alone.
- **Hooks stay opt-in**, as Rust's pre-push hook is ([rustc-dev-guide](https://rustc-dev-guide.rust-lang.org/building/suggested.html)). CI enforces, and a hook can be bypassed anyway.

### Time budgets

A budget is the median (p50) of a job's last 20 runs in its own tier, on GitHub-hosted runners with warm caches: pull-request runs for the Quick and Pre-merge suites, merge-queue runs (manual-dispatch runs while the repository is private) and nightly and release runs for the Release suite. A hard cap per job stops hangs.
> Amended by [ADR 0135](0135-while-the-repository-is-private-the-owners-machine-runs-the-pull-request-tier-in-a-throwaway-linux-vm-and-private-fuzzing-nightly.md): while the pull-request tier runs on the owner's machine, its Quick and Pre-merge budgets are measured on its runs there, each job from its start and "all green" from the first job's start.

| Suite | Where | Budget (p50) | Hard cap |
|---|---|---|---|
| Quick | local, a change to one crate, after an incremental build | ≤ 60 s (guidance, not measured) | none |
| Quick | CI, `just quick all` | test run ≤ 2 min, job ≤ 10 min | job 20 min |
| Pre-merge | pull-request tier | "all green" ≤ 20 min after the push, each job ≤ 15 min | job 30 min |
| Release | merge queue | ≤ 60 min per group, each job ≤ 45 min | job 90 min, queue 120 min |
| Release | nightly and release pipelines | each job ≤ 3 h | job 5 h, under GitHub's 6 h |

> Agent decision of 2026-10-23, as built: while the repository is private, the hosted Release cells run on GitHub's 2-vCPU standard runners, where one job holds the whole cell, and the 90-minute hard cap cancelled three of them in that day's run (`linux-x64-gnu`, `linux-x64-musl` and `linux-arm64-gnu`) before their last steps. Measured there: the cell's build took 22 to 30 minutes, the Rust tests 31 to 38, and the gnu cells' base and head benchmark builds 44 (arm64) and over 55 (x64) minutes. Until the cells are sharded, their hard caps are 300 minutes for `linux-x64-gnu` (it adds the benchmarks, TLC, AddressSanitizer, V8, ZAP and LazyFS), 210 for `linux-arm64-gnu` (the benchmarks), 150 for `windows-x64` and 120 for the musl cells, below GitHub's 360 per job. A cap stops a hang; the 45-minute budget and its breach issue are unchanged.

- **Per-test caps stop hangs. They never measure speed.**
  - nextest's `quick` profile marks a test slow after 1 s and kills it at 10 s. Its `ci` profile marks a test slow after 30 s and kills it at 5 min.
  - Vitest, Playwright and pytest (`pytest-timeout`) get the same two levels: 10 s in the Quick suite, 5 min above it.
- **No run fails for being slow within its cap.** Benchmark suites on shared runners vary by about 2.7% from run to run (CodSpeed's own measurement, in the performance-testing findings), and whole jobs vary more with cache hits and queueing. A timing gate on one run would be flaky; a gate on the trend is not.
- **A breach opens an issue.** A budget check, run nightly and at the end of every manual-dispatch Release run (so it runs while the repository is private), reads the durations from the results store ([Flakiness policy and checks that are non-deterministic by nature](https://github.com/goodfetch/goodfetch/issues/280) decides it; until it exists, the check reads the GitHub Actions API). When a budget is breached, it opens or updates one issue per suite or job, labelled `needs-triage`. The fix comes within two weeks:
  - shard the job;
  - speed up the tests;
  - or move a test to a later suite, with the reason in its pull request.

  > Agent decision, accepted by the owner on 2026-10-21 ([#334](https://github.com/goodfetch/goodfetch/issues/334)): the results store exists, so `xtask budgets` reads it rather than the GitHub Actions API. The Release suite holds no secret, so the check runs in the store's ingest that follows each manual-dispatch Release run rather than in the run itself. A median is taken over the last 20 successful runs, or the fewer there are; a suite's "all green" or group time is measured on first attempts only. A tier missing from TESTING.md's table is reported and fails nothing.
  >
  > Superseded in part by ADR 0135's amendment of 2026-10-22: the check is the `budgets` job at the end of each manual-dispatch Release run, which holds the store's token, the Actions secret `TEST_RESULTS_TOKEN` (agent note of 2026-10-23, as built).

  If the pull-request tier runs a stress run of new and changed tests ([Flakiness policy and checks that are non-deterministic by nature](https://github.com/goodfetch/goodfetch/issues/280) decides it), that run counts against the Pre-merge budget. It is sharded to fit, and its run count is never lowered to meet a budget.
- **Performance budgets of the product** (instruction counts, allocations, startup, overhead) are not these budgets. [Performance regression testing](https://github.com/goodfetch/goodfetch/issues/284) decides them.

### Sharding and caching

- **Build once per target, run in shards.** `cargo nextest archive` builds a target's tests once, and `--partition hash:m/n` splits them across jobs: adding a test never moves another. Vitest and Playwright use `--shard`. pytest splits across processes with `pytest-xdist` inside one job. Shards are deterministic, never timing-based.
- **Shard counts are set so each job meets its budget.** A breach issue raises them.
- **Caches are saved only from `main`**, as ADR 0032 says, and since no tests run there after a merge, a **cache-warming job** runs on every push to `main`. It builds the pull-request tier's Linux dependencies and tests without running them, and saves `Swatinem/rust-cache`, the V8 build cache, the pnpm store and uv's cache. It uses the pull-request jobs' rust-cache `shared-key` and the same profiles (`ci-test`, nextest's archive), since rust-cache's default key includes the job id. The other targets' caches are saved by the nightly and by manual-dispatch Release runs on `main`.
  > Amended by [ADR 0135](0135-while-the-repository-is-private-the-owners-machine-runs-the-pull-request-tier-in-a-throwaway-linux-vm-and-private-fuzzing-nightly.md): while the repository is private, it runs on the owner's machine in a VM and writes a cache folder there, which pull-request VMs mount read-only; the pull-request tier uses no GitHub cache, and hosted Linux jobs restore the caches manual-dispatch Release runs save.
  - A pull request restores its base branch's caches, and a run restores the default branch's ([GitHub's cache access rules](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching)). A cache saved in the merge queue would sit on its temporary `gh-readonly-queue/…` branch, which pull requests can't restore (inferred from those rules, which don't name merge-queue branches), so the merge queue saves none.
  - The pnpm store and uv's cache are keyed by their lockfiles.
  - GitHub caps a repository's caches at 10 GB and evicts beyond it, and removes entries unused for 7 days ([same page](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching)). The pull-request tier's Linux caches take priority: the nightly and dispatch saves of other targets are kept only while they fit beside them. Raising the limit is billed, so it is a paid test resource the owner approves. The Goodfetch Chromium, container images and released binaries are fetched by digest and cached by digest.
    > Agent decision, accepted by the owner on 2026-10-21 ([#334](https://github.com/goodfetch/goodfetch/issues/334)): spec #325 has the cache-warming job delete the least recently used caches that are not pull-request Linux caches once the total passes 9 GB. While the repository is private that job runs on the owner's machine, whose tokens hold no write scope (ADR 0135), and only Release runs on `main` save GitHub caches, so the eviction (`xtask evict-caches`) runs in a hosted `caches` job at the end of each Release run on `main`. A pull-request cache is known by its key, which starts with `pull-request-` (or `v0-rust-pull-request` under rust-cache), and a Linux one names `Linux`.
- **Test builds in CI use a `ci-test` Cargo profile** that inherits `test` with `debug = "line-tables-only"`, as uv and Materialize do. Backtraces keep file and line, and builds and the 14 GB runner disk carry less.
  > Note (agent decision, as built), 2026-11-04, [#2050](https://github.com/goodfetch/goodfetch/issues/2050): `dev` and `test` stay Cargo's for the workspace's own crates, but the dependencies a crawl runs on each page build at `opt-level = 3` in `dev`, beside SHA-256, literal search and DuckDB: the HTML parser and its DOM (`html5ever`, `markup5ever`, `tendril`, `string_cache`, `dom_query`), selector matching (`selectors`, `cssparser`), the Filter-list engine (`adblock`), readability (`dom_smoothie`), `serde_json`, `jsonschema`, and the C sources of mimalloc, SQLite and zstd, which `cc` builds at the profile's opt-level. Unoptimized, they held most of the Foxhound Daemon soak's running samples, about 33 ms of CPU a page, so its 40 crawls followed the host's load: 231 to 265 s at a load of 30 to 65, against the five-minute kill, though nothing in them waits. Built optimized, the soak took 76 to 97 s at a mean load of 15 to 25 (three runs), where `main` took 108 to 117 s at a mean load of 16 (two runs). The overrides reach `test` and `ci-test` through `inherits`, never `release`, `ci` or `bench`, so `just counts`, `just perf` and `just compare` build what they built before. The per-page work itself is spec #1577's to cut.

### Reports

- **Every runner writes JUnit XML in every run, local or CI:** nextest's `junit` output in each profile, Vitest's and Playwright's `junit` reporters, and pytest's `--junitxml`.
- **Every CI job uploads its JUnit files as an artifact, pass or fail.** The artifact is named `junit-<suite>-<tier>-<os>-<arch>[-<runtime>]-<shard>`.
- **Each failing job writes a summary** of its failed tests to the job's page.
- **Where results are kept over time and shown** is decided by [Flakiness policy and checks that are non-deterministic by nature](https://github.com/goodfetch/goodfetch/issues/280), which reads these files. The budget check reads its durations from there.

## Considered Options

- **The full matrix only at release time** (nightly and release candidates), as the owner's wording read literally: rejected. Failures on macOS, Windows, arm64 or a runtime would land on `main`, which ships nightly. ADR 0057's increase check and ADR 0032's touched-Sinks rule both rely on a full run before the merge.
- **The full matrix on every pull request:** rejected. The free plan's 5 macOS jobs would queue every push, and authors would wait on platforms their change rarely breaks. Rust and Cargo run Linux before the queue and everything in it.
- **A fourth suite named after the nightly:** rejected. The nightly runs the Release suite again, plus checks that are in no suite. A suite is defined by what it tests, not by when it runs.
- **Mandatory pre-commit hooks** (pre-commit or prek installed for everyone): rejected. CI is the gate, a hook can be skipped with `--no-verify`, and a slow hook teaches people to skip it. Git's own `core.hooksPath` needs no extra tool.
- **A time limit that fails a single run** (a suite over budget by one second fails): rejected. Wall time on shared runners is noisy, and this gate would be flaky. The p50 trend with an issue holds the budget without that.
- **Timing-based sharding** (splitting by recorded durations): rejected. Which tests share a shard would change from run to run, so an order-dependent failure would come and go.
- **Codecov Test Analytics or Trunk for results:** not chosen here. The results store belongs to [Flakiness policy and checks that are non-deterministic by nature](https://github.com/goodfetch/goodfetch/issues/280). Both keep results on a vendor's servers, and Codecov keeps them for 60 days ([docs](https://docs.codecov.com/docs/test-analytics)).

## Consequences

- **Amends ADR 0032.** Its CI tiers keep their content and now run suites: the pull-request tier runs the Pre-merge suite, the merge queue the Release suite, and `main` reruns nothing after the merge. The owner's "release (full matrix)" is the Release suite, and it gates every merge. ADR 0032 carries a one-line amendment note at its CI tiers.
- **Adds to ADR 0032:**
  - the nightly runs the Release suite again on `main`'s head before its own jobs;
  - every release candidate, stable release and patch runs the Release suite before its gates;
  - while the repository is private, the Release suite runs on manual dispatch and passes before each milestone's owner test and before any release;
  - the merge queue's settings, the budgets, sharding by hash, the `ci-test` profile and JUnit artifacts.
- **Adds to ADR 0057:** its increase checks and the pull request's estimate read `main`'s per-target sizes from the report the merging merge-queue run saved (or a manual-dispatch run on `main` while the repository is private), since `main` builds nothing per target after a merge.
- **Adds to ADR 0037:** area 1 (Repository, CI and release) builds this decision, so its row lists ADR 0061.
- **Changes area 1's spec** ([Spec: Repository, CI and release (V1 area 1)](https://github.com/goodfetch/goodfetch/issues/130)):
  - its CI tiers name the suites they run;
  - `just` gets the recipes above;
  - `xtask affected` also serves local runs, diffing the working tree, and emits nextest filtersets;
  - "Cargo's `dev` and `test` profiles stay as they are" holds, with a `ci-test` profile added beside them;
  - the cache-warming job on pushes to `main`;
  - `main`'s size figures come from the size report of the merge-queue run whose tree became `main`'s head, as its `xtask size` section already says of "its own merge-queue tier build".
- **[`xtask affected`: change detection for the pull-request tier](https://github.com/goodfetch/goodfetch/issues/134)** gains scope: a local mode that diffs the working tree, uncommitted changes included, against the merge base with `origin/main`; nextest `rdeps(…)` filtersets; and the package lists Vitest and pytest run. Once it lands, `just quick` and `just pre-merge` switch from the whole workspace to what changed.
- **[Workspace skeleton and the "all green" pull-request check](https://github.com/goodfetch/goodfetch/issues/132)** creates `just fmt`, `lint`, `build`, `test`, `quick`, `pre-merge`, `release`, `release-ci` and `hooks`, plus nextest's `quick` and `ci` profiles with their caps and JUnit output. These recipes meet its criterion "`just` recipes cover build, test, lint and format …, and CI calls the same recipes". Its criterion "`rust-cache` restores on pull requests and saves only from `main`" is met by the cache-warming job.
- **The "Test infrastructure" spec** builds:
  - the budget check and its issue;
  - the job summaries;
  - the workflows' artifact naming;
  - the local skip list of each recipe;
  - mold for Linux test links and a Dev Drive on Windows, which the Rust tooling findings suggest;
  - each runner's quick selection, once [Test layout, naming, shared helpers and the test-first process](https://github.com/goodfetch/goodfetch/issues/277) names it.
