# Three test suites, each run by one `just` command under a time budget, and the release suite gates every merge

Goodfetch's tests run in three suites, each a superset of the one before: the **Quick suite**, the **Pre-merge suite** and the **Release suite**. Each has one `just` command and a time budget. The Release suite holds the full platform, runtime and browser matrix of ADR 0024, and it runs before every merge, in the merge queue, as well as on `main` every night and for every release.

The owner asked for "tiered suites by speed: quick (every change), pre-merge, and release (full matrix), each run by one local command, each with a time budget". ADR 0032 runs the full matrix in the merge queue, so read literally the two conflict: the owner's words put the full matrix in "release", and ADR 0032 runs it before every merge. That is conflict 4 on the test strategy map. The evidence says to keep the matrix before the merge, and to call it the Release suite:

- **Peers that ship on many platforms gate merges on them.** Rust's pull-request builds are five Linux jobs of about 40 minutes, and its merge builds run "tens of CI jobs ... across operating systems and targets", taking about 2 hours ([rustc-dev-guide](https://rustc-dev-guide.rust-lang.org/tests/ci.html)). Cargo's merge queue runs macOS arm64 and Windows x64 and arm64 test jobs of 14 to 21 minutes ([findings](https://github.com/Jbmanllr/automan/blob/research/rust-test-tooling/rnd/research/2026-10-13-rust-test-tooling.md), §12.3).
- **A failure found after the merge costs more.** It lands on `main`, which carries the nightly channel and from which release branches are cut (ADR 0032), and someone must bisect it.
- **Other decisions need the matrix before the merge.** ADR 0057's increase check compares each target's build with `main`'s, and ADR 0032 lets a pull request run only the Sinks it changes because the merge queue then runs them all.
- **The merge queue doesn't hold the author up.** Once queued, the pull request merges or comes back without anyone waiting on it. The free plan's 20 concurrent jobs, 5 of them macOS, are why the matrix runs there and not on every pull request ([GitHub Actions limits](https://docs.github.com/en/actions/reference/limits)).
- **Moving the matrix to release time only** would put macOS, Windows, arm64 and every runtime past the merge. Their failures would surface at the nightly or the release candidate, after other changes had built on them.

Decided in [Test suites, local commands and time budgets](https://github.com/Jbmanllr/automan/issues/276), with the agent answering on the owner's delegation. It rests on [Rust test tooling in 2026](https://github.com/Jbmanllr/automan/issues/263), [Performance regression testing in CI](https://github.com/Jbmanllr/automan/issues/272), [Flaky tests without retries](https://github.com/Jbmanllr/automan/issues/270) and [Upgrade and version-mismatch testing](https://github.com/Jbmanllr/automan/issues/271). The rules every test follows are in `TESTING.md`.

## Decision

### The three suites

Each suite runs everything the suite before it runs.

- **Quick suite**: formatting and lint checks, and the in-process tests, of what a change touches.
  - Rust: unit tests in `src/` (nextest's `kind(lib)`, `kind(bin)` and `kind(proc-macro)`), never `tests/it`.
  - TypeScript and Python: each runner's in-process selection, which [Test layout, naming, shared helpers and the test-first process](https://github.com/Jbmanllr/automan/issues/277) names.
  - The committed format fixtures, one per format version (the upgrade research's fit; [Upgrade and version-mismatch testing](https://github.com/Jbmanllr/automan/issues/287) decides their shape).
  - `xtask codegen --check` and `xtask affected`'s own tests.
  - Nothing that starts a process, a browser or a container, or binds a socket.
- **Pre-merge suite**: what ADR 0032's pull-request tier runs, on one platform. That is the Quick suite plus:
  - every crate's `tests/it` binary;
  - the conformance suites against the local Proving ground with the Goodfetch Chromium;
  - the touched Sinks' suites against their emulators and containers;
  - the touched SDKs, adapters and Integrations on one runtime each;
  - the Console's component and end-to-end tests on Chromium;
  - the docs build and examples when docs change;
  - the static checks ADR 0032 lists.
- **Release suite**: everything the Pre-merge suite runs, for every package rather than only the touched ones, across the full matrix of ADR 0024:
  - every platform: Linux x64 and arm64 on glibc and musl, macOS arm64 and Intel, Windows x64 and arm64;
  - every runtime: every supported Node line, Bun, Deno 2, and every supported CPython with its free-threaded build;
  - browser conformance on every OS;
  - every Sink's suite;
  - the deployment-target smoke runs that CI hosts (ADR 0049);
  - the tests that need released binaries, such as version skew and rolling upgrades (decided by [Upgrade and version-mismatch testing](https://github.com/Jbmanllr/automan/issues/287)).

Checks that are non-deterministic by nature are not in any suite: real SaaS Sinks, live models, the online Proving ground, real vendors, Chrome's beta and dev channels, and the benchmark. They keep the tiers ADRs 0026, 0032 and 0043 give them. How they report, and when they block, is decided by [Flakiness policy and checks that are non-deterministic by nature](https://github.com/Jbmanllr/automan/issues/280).

### Tiers run suites

| CI tier (ADR 0032) | What it runs |
|---|---|
| Pull request | The Quick suite first, as its own job, for the first signal. Then the rest of the Pre-merge suite on Linux, selected by `xtask affected`. "all green" is the only required check. |
| Merge queue (`merge_group`) | The Release suite. "all green" there is required too. |
| `main` after a merge | Nothing more: the merge queue already ran the Release suite on the same tree. |
| Nightly | The Release suite again on `main`'s head, then the nightly-only checks and the nightly publish. A second run of the same commit is what result history needs (the flaky-tests findings). |
| Release candidate, stable release, patch | The Release suite on the release branch's commit, then that release's gates (ADR 0032). |

- **Until the merge queue exists** (the repository is private, ADR 0032), the Release suite runs on manual dispatch. It must pass on the commit before each milestone's owner test (ADR 0058), and before any release.
- **Merge queue settings:** "Only merge non-failing pull requests" on, and a status check timeout of 120 minutes. Group size and build concurrency are set so the queue keeps up with merges, within the free plan's 5 concurrent macOS jobs.

### One `just` command per suite

These are the recipes that [Workspace skeleton and the "all green" pull-request check](https://github.com/Jbmanllr/automan/issues/132) creates. CI calls the same recipes.

| Recipe | What it does |
|---|---|
| `just quick` | The Quick suite over what changed since the merge base with `origin/main`, uncommitted changes included. |
| `just quick all` | The Quick suite over the whole workspace. |
| `just pre-merge` | The Pre-merge suite over what changed, on the host OS. |
| `just pre-merge all` | The Pre-merge suite over everything, on the host OS. |
| `just release` | The host OS's cell of the Release suite, with every runtime installed on the host. |
| `just release-ci` | Dispatches the Release suite's workflow on the current branch's pushed head (`gh workflow run`) and follows it to the end (`gh run watch`). Its exit code is the run's verdict. |
| `just fmt` | Formats Rust, TypeScript and Python in place. |
| `just lint` | Checks formatting, then runs Clippy, oxlint, ruff, pyright and the other linters. |
| `just build` | Builds the workspace. |
| `just test <suite> [packages]` | Runs one suite's tests alone, without lints, for Rust, TypeScript or Python packages. CI jobs call this for their part. |
| `just hooks` | Opt-in: points `core.hooksPath` at the committed `.githooks/`. Its pre-commit hook checks formatting, and its pre-push hook runs `just quick`. |

- **`xtask affected` is the one source of what changed**, in CI and locally. Locally it diffs the working tree against the merge base. It gives nextest an `rdeps(…)` filterset, so crates that depend on a changed crate are tested too, and gives Vitest and pytest their packages.
- **A local suite that can't run a part on this host says so.** For example, the sink containers on a machine without a container engine, or another OS's cell. The recipe lists every part it skipped, with the reason, after its results. Its exit code covers what ran. CI is the verdict.
- **The reference host is never reached from a recipe.** `just release-ci` runs on GitHub-hosted runners only. The benchmark keeps ADR 0025's own command and ADR 0032's trigger rule: tags and manual dispatch by the owner.
- **Hooks stay opt-in**, as Rust's pre-push hook is ([rustc-dev-guide](https://rustc-dev-guide.rust-lang.org/building/suggested.html)). CI enforces, and a hook can be bypassed anyway.

### Time budgets

A budget is measured as the median (p50) over the last 20 runs on `main` and in the merge queue, on GitHub-hosted runners with warm caches. A hard cap per job stops hangs.

| Suite | Where | Budget (p50) | Hard cap |
|---|---|---|---|
| Quick | local, a change to one crate, after an incremental build | ≤ 60 s (guidance, not measured) | none |
| Quick | CI, `just quick all` | test run ≤ 2 min, job ≤ 10 min | job 20 min |
| Pre-merge | pull-request tier | "all green" ≤ 20 min after the push, each job ≤ 15 min | job 30 min |
| Release | merge queue | ≤ 60 min per group, each job ≤ 45 min | job 90 min, queue 120 min |
| Release | nightly and release pipelines | each job ≤ 3 h | job 5 h, under GitHub's 6 h |

- **Per-test caps stop hangs. They never measure speed.**
  - nextest's `quick` profile marks a test slow after 1 s and kills it at 10 s. Its `ci` profile marks a test slow after 30 s and kills it at 5 min.
  - Vitest, Playwright and pytest (`pytest-timeout`) get the same two levels: 10 s in the Quick suite, 5 min above it.
- **No run fails for being slow within its cap.** Shared runners vary by about 2.7% in wall time (the performance-testing findings), so a timing gate on one run would be flaky. A timing gate on the trend is not.
- **A breach opens an issue.** A nightly budget check reads the durations from the results store ([Flakiness policy and checks that are non-deterministic by nature](https://github.com/Jbmanllr/automan/issues/280) decides it; until it exists, the check reads the GitHub Actions API). When a budget is breached, it opens or updates one issue per suite or job, labelled `needs-triage`. The fix comes within two weeks:
  - shard the job;
  - speed up the tests;
  - or move a test to a later suite, with the reason in its pull request.
- **Performance budgets of the product** (instruction counts, allocations, startup, overhead) are not these budgets. [Performance regression testing](https://github.com/Jbmanllr/automan/issues/284) decides them.

### Sharding and caching

- **Build once per target, run in shards.** `cargo nextest archive` builds a target's tests once, and `--partition hash:m/n` splits them across jobs: adding a test never moves another. Vitest and Playwright use `--shard`. pytest splits across processes with `pytest-xdist` inside one job. Shards are deterministic, never timing-based.
- **Shard counts are set so each job meets its budget.** A breach issue raises them.
- **Caches** follow ADR 0032: `Swatinem/rust-cache` and the V8 build cache save only from `main`. The pnpm store and uv's cache are keyed by their lockfiles. The Goodfetch Chromium, container images and released binaries are fetched by digest and cached by digest.
- **Test builds in CI use a `ci-test` Cargo profile** that inherits `test` with `debug = "line-tables-only"`, as uv and Materialize do. Backtraces keep file and line, and builds and the 14 GB runner disk carry less.

### Reports

- **Every runner writes JUnit XML in every run, local or CI:** nextest's `junit` output in each profile, Vitest's and Playwright's `junit` reporters, and pytest's `--junitxml`.
- **Every CI job uploads its JUnit files as an artifact, pass or fail.** The artifact is named `junit-<suite>-<tier>-<os>-<arch>[-<runtime>]-<shard>`.
- **Each failing job writes a summary** of its failed tests to the job's page.
- **Where results are kept over time and shown** is decided by [Flakiness policy and checks that are non-deterministic by nature](https://github.com/Jbmanllr/automan/issues/280), which reads these files. The budget check reads its durations from there.

## Considered Options

- **The full matrix only at release time** (nightly and release candidates), as the owner's wording read literally: rejected. Failures on macOS, Windows, arm64 or a runtime would land on `main`, which ships nightly. ADR 0057's increase check and ADR 0032's touched-Sinks rule both rely on a full run before the merge.
- **The full matrix on every pull request:** rejected. The free plan's 5 macOS jobs would queue every push, and authors would wait on platforms their change rarely breaks. Rust and Cargo run Linux before the queue and everything in it.
- **A fourth suite named after the nightly:** rejected. The nightly runs the Release suite again, plus checks that are in no suite. A suite is defined by what it tests, not by when it runs.
- **Mandatory pre-commit hooks** (pre-commit or prek installed for everyone): rejected. CI is the gate, a hook can be skipped with `--no-verify`, and a slow hook teaches people to skip it. Git's own `core.hooksPath` needs no extra tool.
- **A time limit that fails a single run** (a suite over budget by one second fails): rejected. Wall time on shared runners is noisy, and this gate would be flaky. The p50 trend with an issue holds the budget without that.
- **Timing-based sharding** (splitting by recorded durations): rejected. Which tests share a shard would change from run to run, so an order-dependent failure would come and go.
- **Codecov Test Analytics or Trunk for results:** not chosen here. The results store belongs to [Flakiness policy and checks that are non-deterministic by nature](https://github.com/Jbmanllr/automan/issues/280). Both keep results on a vendor's servers, and Codecov keeps them for 60 days ([docs](https://docs.codecov.com/docs/test-analytics)).

## Consequences

- **Amends ADR 0032.** Its CI tiers keep their content and now run suites: the pull-request tier runs the Pre-merge suite, the merge queue the Release suite, the nightly the Release suite again plus its own checks, and every release the Release suite plus its gates. The owner's "release (full matrix)" is the Release suite, and it gates every merge. ADR 0032's text is amended in place.
- **Adds to ADR 0032:** the merge queue's settings, the budgets, sharding by hash, the `ci-test` profile, JUnit artifacts, and the release suite on manual dispatch before each milestone's owner test while the repository is private.
- **Changes area 1's spec** ([Spec: Repository, CI and release (V1 area 1)](https://github.com/Jbmanllr/automan/issues/130)):
  - its CI tiers name the suites they run;
  - `just` gets the recipes above;
  - `xtask affected` also serves local runs, diffing the working tree, and emits nextest filtersets;
  - "Cargo's `dev` and `test` profiles stay as they are" holds, with a `ci-test` profile added beside them.
- **[Workspace skeleton and the "all green" pull-request check](https://github.com/Jbmanllr/automan/issues/132)** creates `just fmt`, `lint`, `build`, `test`, `quick`, `pre-merge`, `release`, `release-ci` and `hooks`, plus nextest's `quick` and `ci` profiles with their caps and JUnit output. Each of its acceptance criteria ("`just` recipes cover build, test, lint and format …, and CI calls the same recipes") names these recipes.
- **The "Test infrastructure" spec** builds:
  - the budget check and its issue;
  - the job summaries;
  - the workflows' artifact naming;
  - the local skip list of each recipe;
  - mold for Linux test links and a Dev Drive on Windows, which the Rust tooling findings suggest;
  - each runner's quick selection, once [Test layout, naming, shared helpers and the test-first process](https://github.com/Jbmanllr/automan/issues/277) names it.
