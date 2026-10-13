# Tests are never retried, and checks of what Goodfetch does not pin run as Live checks

No test is retried, anywhere: not by a runner, not by a workflow, not by a loop in the test, not by re-running a red job. Flaky tests are caught before merge by running every new or changed test 20 times in random order, and after merge by comparing results of the same commit kept in a results store. A flaky test found after merge is reverted with the change that brought it, or quarantined the same day with an issue that comes first for every agent and blocks its milestone and every release until a fix passes 200 runs. A check whose inputs Goodfetch does not pin (a vendor, a real SaaS account, a live model, a moving Chrome channel) is never a test: it is a **Live check**, run outside the suites, with a pass, fail or inconclusive outcome and its own rule for what it blocks.

The owner asked for "no flaky tests and no retries". `STYLE.md` said "A flaky test is quarantined with an issue, never retried silently", which allowed a retry that is not silent and gave a quarantine no owner, no deadline and no end (conflict 3 on the map). Comparable projects split this way:
- **Most large organizations retry** before merge: Google, Chromium, Meta, GitHub and Uber. Chromium's reason is its ~20,000 existing flaky tests, without which "pretty much no CL would ever pass". A new repository has none.
- **Kubernetes has not retried since 2019-12-13** ("zero-flake" policy, confirmed in 2023): a flaky test gets a `[Flaky]` name tag and a `priority/critical-urgent` issue in the current milestone, and "is more important than anything else you're doing" ([flaky-tests.md](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-testing/flaky-tests.md)).
- **New tests are where flaky tests come from**: 78% are flaky the first time they are written (Luo et al. 2014). Chromium runs each new test 20 times and fails a flaky one; Mozilla runs each added or changed test 20 times, then backs out a flaky test landed in the last 7 days.
- **A quarantine works only with an owner, a clock and a place where the test still runs** (Mozilla's 2 + 2 business days, Uber's ticket deadline, Chromium's reviver builders).
- **Checks against systems a project does not control run outside its blocking suites** everywhere: Kubernetes' Informing jobs, Chromium's FYI builders, Mozilla's tier-1 ban on sites "whose content we do not control", LangChain's daily live-credential tests.

Decided in [Flakiness policy and checks that are non-deterministic by nature](https://github.com/Jbmanllr/automan/issues/280), on the facts of [Flaky tests without retries](https://github.com/Jbmanllr/automan/issues/270) ([findings](https://github.com/Jbmanllr/automan/blob/research/flaky-tests/rnd/research/2026-10-13-flaky-tests.md)), [Rust test tooling in 2026](https://github.com/Jbmanllr/automan/issues/263), [Testing AI features without flaky tests](https://github.com/Jbmanllr/automan/issues/268) and [Performance regression testing in CI](https://github.com/Jbmanllr/automan/issues/272). The rules every test follows are in `TESTING.md`.

The suites are `quick`, `pre-merge` and `release`, run in the CI tiers of ADR 0032 (pull request, merge queue, `main`, nightly, release candidate), as [Test suites, local commands and time budgets](https://github.com/Jbmanllr/automan/issues/276) defines them. This ADR uses those names and decides none of them.

## Decision

### What a test is

- **A test pins every input:** the commit, the Proving ground seed, the `Entropy` seed (ADR 0059), the browser build, container images by digest, recorded model exchanges. A **flaky test** passes and fails with the same inputs (Google's definition).
- **A check with an input that moves without a commit is a Live check, never a test.** It never runs in a suite.

### No retries

- **Every runner keeps zero retries, and every lock the runner offers is on:**
  - nextest: `retries = 0` and `flaky-result = "fail"` in every profile, so even a `--retries` passed by hand cannot turn red into green;
  - Vitest: `retry: 0`, and no `retry` option on any test or suite;
  - Playwright Test: `retries: 0`, `failOnFlakyTests: true`, and no `retries` in `test.describe.configure`;
  - pytest: no rerun plugin (pytest-rerunfailures, flaky) installed, and no `flaky` marker.
- **No workflow step retries a test or a check** (a retry action, a shell loop, `continue-on-error` on a test step).
- **No test loops until it passes.** Waiting on a condition with a deadline is not a retry: Playwright's auto-waiting and `expect.poll`, waiting for a CDP event, polling a state the test can name. Repeating the action under test until it succeeds is.
- **A check in the `quick` suite refuses all of the above** in runner configurations, workflow files, lockfiles and test sources. It is one subcommand of `xtask`.
- **Downloads before the deny of ADR 0043 may retry** (toolchains, dependencies, images, the artifact mirror). They test nothing.
- **Re-running a failed job is allowed only when no test step failed in it** (a runner lost, a download failed). The "all green" job reads the earlier attempts of its run through GitHub's attempts API and fails when any of them has a failed test step. Every attempt's results are kept, so a pass on a re-run would also be caught as a flaky test.

### Before merge

- **Every new or changed test runs 20 times in all** in the pull-request tier, and one failure fails the pull request.
  - *New* means its ID is absent from the base commit's test listing. *Changed* means its file changed in the pull request (Vitest `--changed` and Playwright `--only-changed` also take tests whose imports changed).
  - nextest `--stress-count`, Vitest `--repeats`, Playwright `--repeat-each`, pytest-repeat `--count`.
  - The run is sharded to fit the `pre-merge` budget; the count is never lowered, and no label or footer skips it.
- **A pull request that takes a test off the quarantine list runs it 200 times.** About 170 runs give 95% confidence that a passing test is not flaky (Gruber et al. 2021).
- **Tests run in random order wherever the runner allows it**, with the seed printed:
  - pytest: pytest-randomly, on when installed;
  - Vitest: `sequence.shuffle.tests`. `sequence.shuffle.files` joins once Vitest's [#11340](https://github.com/vitest-dev/vitest/issues/11340) makes a seed reproduce file order; files already run isolated;
  - Playwright Test: `--shuffle` (1.64) with `fullyParallel`;
  - nextest: no shuffle exists, and each test runs in its own process.
  - The seed comes from the commit's tree hash in the pull-request, merge-queue and `main` tiers, so the same commit always runs in the same order, and a fresh seed each night. Either can be passed to reproduce an order.
- **Shared state is reset by the runner:** Vitest `restoreMocks`, `unstubEnvs` and `unstubGlobals` are on, since shared mocking state is the main cause of order dependence found in JavaScript (Hashemi et al. 2025).
- **Leaks and hangs fail:** nextest's leaky-test result is `fail`, every test has the hang timeout of its suite (set by [Test suites, local commands and time budgets](https://github.com/Jbmanllr/automan/issues/276)), and Vitest's `detectAsyncLeaks` runs in the nightly tier.
- **No test asserts elapsed real time, a rate or a duration.** Those are measurements, decided by [Performance regression testing](https://github.com/Jbmanllr/automan/issues/284).

### After merge: the results store and flaky-test detection

This settles the map's fog "Where test results are kept over time and shown".

- **Every runner writes JUnit XML in every tier and every attempt, pass or fail,** uploaded as an artifact (named by the suites decision).
- **An ingest workflow, triggered by each finished run, copies them into the results store.** It runs none of the product, so ADR 0043's deny does not apply to it.
  - The store is a separate repository of the project's organization, `test-results`, private until the main repository is public. It holds one compressed file per workflow run attempt: the JUnit files as written, and one JSON Lines record per test run.
  - A record carries the test ID, runner, suite, tier, job, OS, architecture, runtime, shard, commit, tree hash, run, attempt, outcome, duration, the order seed and the `Entropy` seed.
  - It keeps everything. The records are small, and trends need years. JUnit artifacts in Actions keep GitHub's retention.
- **Allure Report 3 shows the history.** A nightly job builds it from the store (its JUnit reader and `history.jsonl`): per-test history, durations, trends and Allure's own flaky mark. It is published on the store's GitHub Pages once the repository is public, and attached to the nightly run before that. Each run also writes a summary of its failures to its job summary.
- **A flaky test is detected two ways, by an `xtask` scan of the store after each ingest:**
  1. **Same inputs, different results:** a test has passed and failed on the same tree hash and the same configuration (OS, architecture, runtime) in any tiers or attempts.
  2. **A diagnostic run:** when a test fails in the merge queue, on `main` or in the nightly tier, a separate job runs it 20 more times on the same commit. Any pass marks it flaky; 20 failures mark it broken, which is an ordinary failure. The diagnostic run never changes any verdict: the red run stays red.
- **One nightly Linux job runs the `pre-merge` suite under a cgroup that halves the runner's CPU and memory.** 46.5% of flaky tests change failure rate with the resources available (Silva et al. 2024), and Uber runs `main` with limited resources for that reason.
- **Durations in the store feed the suites' time budgets and the performance trends.** They never fail a test.

### A test that flakes

- **The scan opens one issue per flaky test**, labelled `flaky-test`, in the current milestone, with the runs, seeds and configurations that differed. A recurrence reopens it.
- **Same day, one of two things happens:**
  - **Revert:** if the test or the code it covers changed in the last 7 days, the pull request that changed it is reverted by a new pull request, and comes back with the fix (Mozilla's rule).
  - **Quarantine:** otherwise, a pull request adds the test to the test quarantine list with its issue.
- **The test quarantine list is one file in the repository**, reviewed like code. Each entry names a test ID, its open `flaky-test` issue and the date. The `quick` suite's check refuses an entry with no open issue, or with a test that no longer exists. Each suite command excludes the listed tests through its runner's own filter.
- **A quarantined test keeps running:** a nightly job runs each one 50 times and writes the results to the store, so the issue shows its failure rate and a fix can be confirmed.
- **`flaky-test` issues come first on the frontier**, before every other ticket of the current milestone.
- **The fix removes the cause.** A longer timeout alone, a sleep or a retry is not a fix. When the cause is in the product (a race, a leak), the product is fixed and the test stays as it was.
- **It leaves the list only through a fix that passes 200 runs.** The issue closes when that pull request merges.
- **A quarantined test blocks:**
  - its milestone, whose `ready-for-human` gate issue (ADR 0058) is ready only once it is the only open issue left;
  - every release: `xtask release` refuses to cut or promote while the list is not empty;
  - the coverage of the feature it tests, which counts as untested in the check of [Feature coverage and traceability](https://github.com/Jbmanllr/automan/issues/279).
- **A test is never deleted by an agent.** A flaky test is fixed, or its unpinned part moves to a Live check. A test whose behaviour left the product is removed by the owner, following the file-removal rule: the agent proposes the exact paths and commands.

### Live checks

- **Each has an owning area and a declared placement.** The ones decided so far:

| Live check | Tier | What it blocks | Decided in |
|---|---|---|---|
| Real SaaS Sink accounts | nightly | nothing directly | ADRs 0026, 0032 |
| Installed Chrome: dev and beta channels | nightly | nothing directly | ADRs 0030, 0032 |
| Public testnets for Paid fetch proofs | nightly | nothing directly | ADR 0045 |
| Clean-up rules' daily channel gate (online Proving ground) | nightly | that night's channel publish | ADR 0047 |
| Online conformance against real vendors | release candidate, Chrome-major and stealth patches | never a release | ADRs 0033, 0043 |
| Benchmark on the reference host | release candidate | the release, by ADR 0025's correctness and Δ rule | ADRs 0025, 0030 |
| Cloud Run deploy | release candidate | nothing directly | ADR 0049 |
| Live-model evaluations | decided by the AI testing ticket | decided by the AI testing ticket | [Testing AI features and agent skills](https://github.com/Jbmanllr/automan/issues/281) |
| OpenSSF Scorecard | nightly | nothing | ADR 0032 |

  "Nothing directly" means only the two-day rule below applies. Stable Chrome majors and Extended Stable are pinned by version in the nightly matrix, so they are tests.
- **A Live check has three outcomes: pass, fail and inconclusive.** Inconclusive means the external side was unavailable (a vendor outage, an unreachable account, an exhausted quota). It is never counted as a pass.
- **Its results go to the results store** under its own tier, `live`, and show in the same report.
- **A fail, or three inconclusive results in a row, opens or updates one issue per Live check**, labelled `live-check`, for its owning area.
- **An open `live-check` issue whose cause is not stated within two business days blocks the promotion of the next minor** until the cause is written down: a Goodfetch bug (fixed like any bug) or an external change (recorded). The Kubernetes Informing rule. Patches are not held, and the online conformance keeps ADR 0043's rule that it never blocks.
- **A measurement declares its trial count, threshold and statistic before it runs** (the lab's method), and every trial counts. A failed trial is never replaced by another. Live-model evaluations use pass^k with Wilson intervals; the values belong to [Testing AI features and agent skills](https://github.com/Jbmanllr/automan/issues/281).

## Considered Options

- **Retries that report the test as flaky but fail anyway** (nextest's `flaky-result = "fail"` with retries on, Playwright's `failOnFlakyTests`) in the gating run: rejected. The verdict is the same, the run costs the extra attempts, and a "flaky" label in a green-looking report trains people to dismiss failures, the complaint Google and Gruber and Fraser record. The diagnostic run gives the same classification in a separate job. The locks stay on, at zero retries.
- **Retrying before merge, as Chromium and Google do**: rejected. Their reason is tens of thousands of existing flaky tests; Goodfetch starts with none, and the owner rules it out.
- **Leaving a flaky test failing until it is fixed (block)**: rejected. It stops every unrelated pull request, which Kubernetes counts as "at least 2 hours of merge time" per recurrence.
- **Quarantine as `STYLE.md` had it, with an issue only**: rejected. With no owner, clock or still-running place, quarantined tests rot; every source that quarantines adds all three.
- **Quarantine held by a service** (Trunk, Datadog, BuildPulse), which turns a red job green after the run: rejected. The verdict then depends on state outside the repository and outside review.
- **A quarantine marker in each test's source** (Kubernetes' `[Flaky]` name tag, Vitest tags, pytest markers): rejected. nextest has no tags, renaming a Rust test breaks its history, and one list gives one check.
- **Datadog Test Optimization for history**: rejected. Its library reports from inside the test process, which ADR 0043's deny blocks, and Rust is not in its library list.
- **Trunk Flaky Tests or Codecov Test Analytics for history**: rejected. Each holds the project's test history in a third-party service, Trunk's free tier stops at five committers, and Codecov only flags. Allure Report 3 (Apache-2.0, release 3.20.1 on 2026-10-06) shows the same history from files the project keeps.
- **ReportPortal**: rejected. It needs a server to run and keep up, for what a static report gives.
- **Keeping results only as Actions artifacts**: rejected. They expire, and nothing compares runs across them.
- **Results on an orphan branch of the main repository**: rejected. Every clone would carry years of results; Deno keeps its benchmark history in a separate repository for that reason.
- **Fewer than 20 stress runs, or a stress run only on request** (Spotify's Flakybot): rejected. 20 is what Chromium and Mozilla settled on; the 200-run bar applies where a known flaky test comes back.
- **A random order seed on every run**: rejected for the merge path. The same commit must give the same order, so a re-run reproduces a failure; the nightly tier explores new orders.
- **Every Live check reports only, Mozilla tier-3 style**: rejected. An unexplained red result would then be ignored; Kubernetes blocks a release until it is explained.

## Consequences

- **Amends `STYLE.md`.** "A flaky test is quarantined with an issue, never retried silently" becomes "A test is never retried; a flaky test is reverted or quarantined the same day, with an issue that comes first and blocks its milestone and every release (ADR 0064)". The rules live in `TESTING.md`.
- **Changes area 1's spec** ([Spec: Repository, CI and release (V1 area 1)](https://github.com/Jbmanllr/automan/issues/130)). User story 65 becomes "a flaky test never retried, reverted or quarantined the same day with an issue that blocks releases". The "all green" job also checks earlier attempts; `xtask` gains the no-retry check, the quarantine-list check, the scan and the release refusal; the ingest workflow and the `test-results` repository join the jobs that run none of the product.
- **Changes area 3's spec** ([Spec: Engine runtime (V1 area 3)](https://github.com/Jbmanllr/automan/issues/213)). "flaky tests quarantined with an issue" becomes "flaky tests never retried, and handled by ADR 0064".
- **Adds to ADR 0032.** The nightly tier also runs the constrained-resource job, the quarantined tests and Allure's report. Before the repository is public the nightly tier is off, so detection relies on the same-commit comparison and the diagnostic runs.
- **Adds to ADR 0058.** `flaky-test` issues sit in the current milestone, so its gate waits for them.
- **ADRs 0025, 0026, 0030, 0033, 0043, 0045, 0047 and 0049 are unchanged** in where their checks run and what they block. This ADR names them Live checks and adds the inconclusive outcome, the results store and the two-day rule for unexplained failures.
- **`docs/agents/issue-tracker.md`** puts `flaky-test` issues first on the frontier and names the `flaky-test` and `live-check` labels.
- **The "Test infrastructure" spec** builds: the `xtask` checks and scan, the stress selection per runner, the order seed, the ingest workflow, the `test-results` repository and its token, the Allure build, the diagnostic and constrained-resource jobs, the quarantine-list format and its exclusion per runner, and the two labels.
