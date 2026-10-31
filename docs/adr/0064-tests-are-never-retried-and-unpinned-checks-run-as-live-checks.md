# Tests are never retried, and checks of what Goodfetch does not pin run as Live checks

No test is retried, anywhere: not by a runner, not by a workflow, not by a loop in the test, not by re-running a red job. Flaky tests are caught before merge by running every new or changed test 20 times in random order, and after merge by comparing results of the same commit kept in a results store. A flaky test found after merge is quarantined the same day, and the change that brought it may also be reverted, with an issue that comes first for every agent and blocks its milestone and every release until a fix passes 200 runs. A flaky test is never removed or weakened to make a suite pass. A check whose inputs Goodfetch does not pin (a vendor, a real SaaS account, a live model, a moving Chrome channel) is never a test: it is a **Live check**, run outside the suites, with a pass, fail or inconclusive outcome and its own rule for what it blocks.

The owner asked for "no flaky tests and no retries". `STYLE.md` said "A flaky test is quarantined with an issue, never retried silently", which allowed a retry that is not silent and gave a quarantine no owner, no deadline and no end (conflict 3 on the map). Comparable projects split this way:
- **Most large organizations retry** before merge: Google, Chromium, Meta, GitHub and Uber. Chromium's reason is its ~20,000 existing flaky tests, without which "pretty much no CL would ever pass". A new repository has none.
- **Kubernetes has not retried since 2019-12-13** ("zero-flake" policy, confirmed in 2023): a flaky test gets a `[Flaky]` name tag and a `priority/critical-urgent` issue in the current milestone, and "is more important than anything else you're doing" ([flaky-tests.md](https://github.com/kubernetes/community/blob/master/contributors/devel/sig-testing/flaky-tests.md)).
- **New tests are where flaky tests come from**: 78% are flaky the first time they are written (Luo et al. 2014). Chromium runs each new test 20 times and fails a flaky one; Mozilla runs each added or changed test 20 times, then backs out a flaky test landed in the last 7 days.
- **A quarantine works only with an owner, a clock and a place where the test still runs** (Mozilla's 2 + 2 business days, Uber's ticket deadline, Chromium's reviver builders).
- **Checks against systems a project does not control run outside its blocking suites** everywhere: Kubernetes' Informing jobs, Chromium's FYI builders, Mozilla's tier-1 ban on sites "whose content we do not control", LangChain's daily live-credential tests.

Decided in [Flakiness policy and checks that are non-deterministic by nature](https://github.com/goodfetch/goodfetch/issues/280), on the facts of [Flaky tests without retries](https://github.com/goodfetch/goodfetch/issues/270) ([findings](https://github.com/goodfetch/goodfetch/blob/research/flaky-tests/rnd/research/2026-10-13-flaky-tests.md)), [Rust test tooling in 2026](https://github.com/goodfetch/goodfetch/issues/263), [Testing AI features without flaky tests](https://github.com/goodfetch/goodfetch/issues/268) and [Performance regression testing in CI](https://github.com/goodfetch/goodfetch/issues/272). The rules every test follows are in `TESTING.md`.

The suites are the Quick, Pre-merge and Release suites, each containing the one before, run in the CI tiers of ADR 0032 (pull request, merge queue, nightly, release pipeline), as [ADR 0061](0061-three-test-suites-each-run-by-one-just-command-under-a-time-budget.md) defines them. This ADR uses those names and decides none of them.

## Decision

### What a test is

- **A test pins every input:** the commit, the Proving ground seed, the `Entropy` seed (ADR 0059), the browser build, container images by digest, recorded model exchanges. A **flaky test** passes and fails with the same inputs (Google's definition).
- **A check with an input that moves without a commit is a Live check, never a test.** It never runs in a suite.

### No retries

- **Every runner keeps zero retries, and every lock the runner offers is on:**
  - nextest: `retries = 0` and `flaky-result = "fail"` in every profile, with no per-test override of either;
  - Vitest: `retry: 0`, and no `retry` option on any test or suite;
  - Playwright Test: `retries: 0`, `failOnFlakyTests: true`, and no `retries` in `test.describe.configure`;
  - pytest: no rerun plugin (pytest-rerunfailures, flaky) installed, and no `flaky` marker.
- **No workflow step retries a test or a check** (a retry action, a shell loop, `continue-on-error` on a test step).
- **No test loops until it passes.** Waiting on a condition with a deadline is not a retry: Playwright's auto-waiting and `expect.poll`, waiting for a CDP event, polling a state the test can name. Repeating the action under test until it succeeds is.
- **A check in the Quick suite refuses all of the above** in runner configurations (per-test overrides included), workflow files, lockfiles and test sources. In workflow files and suite commands it also refuses the flags and variables that undo them: nextest `--retries`, `--flaky-result`, `NEXTEST_RETRIES` and `NEXTEST_FLAKY_RESULT`, Vitest `--retry`, Playwright `--retries`. It is one subcommand of `xtask`.
- **Downloads before the deny of ADR 0043 may retry** (toolchains, dependencies, images, the artifact mirror). They test nothing.
- **Re-running a failed job is allowed only when no test step failed in it** (a runner lost, a download failed). The "all green" job reads the earlier attempts of its run through GitHub's attempts API and fails when any of them has a failed test step. Every attempt's results are kept, so a pass on a re-run would also be caught as a flaky test.

### Before merge

- **Every new or changed test runs 20 times in all** in the pull-request tier, and one failure fails the pull request.
  - *New* means its ID is absent from the base commit's test listing. *Changed* means its file changed in the pull request (Vitest `--changed` and Playwright `--only-changed` also take tests whose imports changed).
  - nextest `--stress-count`, Vitest `--repeats`, Playwright `--repeat-each`, pytest-repeat `--count`.
  - The run is sharded to fit the Pre-merge budget; the count is never lowered, and no label or footer skips it.

> Amended on 2026-10-28 (owner decision of 2026-10-28, adopting option 2 of the test-gate audit): a red stress run stops at the end of its first red iteration, the first run of every selected test in which one failed. One failure has already failed the pull request, and the 3 h 30 the iterations after it took on spec #577's red rounds decided nothing. A green run still runs every test its 20 or 200 times. nextest runs that run's iterations one a command, since its `--max-fail` ends a stress run in the middle of the iteration that reaches it; after a red iteration, no later iteration, runner or count runs. The cost: a second flaky test the later iterations would have caught shows on the next round, and a run that stopped before nextest's last iteration records red, in the gate (ADR 0135), every crate with a selected nextest test, since none of those ran its count.

- **A pull request that takes a test off the quarantine list runs it 200 times.** Gruber et al. (2021) found that Python tests need about 170 runs on average for 95% confidence that a passing test is not flaky.
- **Tests run in random order wherever the runner allows it**, with the seed printed:
  - pytest: pytest-randomly, on when installed;
  - Vitest: `sequence.shuffle.tests`. `sequence.shuffle.files` joins once Vitest's [#11340](https://github.com/vitest-dev/vitest/issues/11340) makes a seed reproduce file order; files already run isolated;
  - Playwright Test: `--shuffle` (1.64) with `fullyParallel`;
  - nextest: no shuffle exists, and each test runs in its own process.
  - The seed comes from the tree hash of the commit under test in every run but one, so the same tree always runs in the same order: in the merge queue, that is the tree that becomes `main`'s head. The constrained test-health run (below) takes a fresh seed. Either seed can be passed to reproduce an order.
- **Shared state is reset by the runner:** Vitest `restoreMocks`, `unstubEnvs` and `unstubGlobals` are on, since shared mocking state is the main cause of order dependence found in JavaScript (Hashemi et al. 2025).
- **Leaks and hangs fail:** nextest's leaky-test result is `fail`, every test has the hang timeout of its suite (set by [ADR 0061](0061-three-test-suites-each-run-by-one-just-command-under-a-time-budget.md)), and the constrained test-health run (below) runs the Vitest tests of the Pre-merge suite with `detectAsyncLeaks`.
- **No test asserts elapsed real time, a rate or a duration.** Those are measurements, decided by [Performance regression testing](https://github.com/goodfetch/goodfetch/issues/284).

### After merge: the results store and flaky-test detection

This settles the map's fog "Where test results are kept over time and shown".

- **Every runner writes JUnit XML in every tier and every attempt, pass or fail,** uploaded as an artifact (named by the suites decision).
- **An ingest workflow, triggered by each finished run, copies them into the results store.** It runs none of the product, so ADR 0043's deny does not apply to it.
  > Amended by [ADR 0135](0135-while-the-repository-is-private-the-owners-machine-runs-the-pull-request-tier-in-a-throwaway-linux-vm-and-private-fuzzing-nightly.md): while the repository is private, it is triggered only by runs of workflows with a hosted job, and one daily hosted job ingests the runs that ran only on the owner's machine.
  - The store is a separate repository, `test-results`, private until the main repository is public. It sits beside the main repository: under the owner's account until the organization exists, then moved into it with the main repository (ADR 0032), which GitHub redirects. It holds one compressed file per workflow run attempt: the JUnit files as written, and one JSON Lines record per test run.
  - A record carries the test ID, runner, suite, tier, job, OS, architecture, runtime, shard, commit, tree hash, run, attempt, outcome, duration, the order seed and the `Entropy` seed. Each job's duration and runner OS are recorded too.
  - It keeps everything. The records are small, and trends need years. JUnit artifacts in Actions keep GitHub's retention.
- **Allure Report 3 shows the history.** The daily test-health job builds it from the store (its JUnit reader and `history.jsonl`): per-test history, durations, trends and Allure's own flaky mark. It is published on the store's GitHub Pages once the repository is public, and attached to the job's run before that. Each run also writes a summary of its failures to its job summary.
- **A flaky test is detected two ways, by an `xtask` scan of the store after each ingest:**
  1. **Same inputs, different results:** a test has passed and failed on the same tree hash and the same configuration (OS, architecture, runtime) in any tiers or attempts.
  2. **A diagnostic run:** when a test fails in the merge queue, in a manual-dispatch Release run, in the nightly tier or in a test-health run, a separate job runs it 20 more times on the same commit and the same runner OS. Any pass marks it flaky; 20 failures mark it broken, which is an ordinary failure. The diagnostic run never changes any verdict: the red run stays red. Diagnostic runs are triggered by failures, not by a tier, so they run whether or not the nightly tier is on; while the repository is private, failed manual-dispatch Release runs are the only failures seen on macOS and Windows. Each diagnostic job times out after 10 minutes.
    > Amended by [ADR 0074](0074-the-goodfetch-chromium-is-gated-where-it-is-built-against-a-control-build-of-the-same-revision.md): the Chromium build gate's diagnostic jobs time out after 60 minutes.
- **Test health runs from M1 on, whether or not the repository is public, with no paid resource.** ADR 0032 turns the nightly tier off while the repository is private, because private repositories pay for hosted-runner minutes. Until it is on, test health is split so that it fits in the account's included minutes, and detection and quarantine still work during M1 to M3:
  - **The daily test-health job**, one scheduled Linux job on `main`'s head, exempt from ADR 0032's rule: it runs each quarantined test 50 times, then the scan and the Allure build. It runs at most 10 quarantined tests, the oldest entries first. Above 10, the milestone is blocked anyway, and a fix still proves itself with its 200 runs. The job times out after 15 minutes.
  - **The constrained test-health run**, weekly and after each manual-dispatch Release run: the Pre-merge suite on Linux under a cgroup that halves the runner's CPU and memory, with a fresh order seed and Vitest's `detectAsyncLeaks`. 46.5% of flaky tests change failure rate with the resources available (Silva et al. 2024), and Uber runs `main` with limited resources for that reason. All its shards together time out after 120 minutes.
  - **A ceiling of 1,000 included minutes a month** covers these jobs and the diagnostic runs, counted from the job durations the ingest records, with GitHub's minute multipliers for macOS and Windows. GitHub Free includes 2,000 a month for private repositories, and the rest goes to the pull-request tier and the manual-dispatch Release runs. Expected use is about 5 minutes a day for the daily job and about 60 per constrained run, some 450 a month. Once the month's count reaches the ceiling, the constrained runs and then the diagnostic runs are skipped until the month ends, and the skip is written to the job summary; the daily job, bounded by its timeout, keeps running.

    > Amended by [ADR 0108](0108-while-the-repository-is-private-the-release-suite-runs-its-macos-cells-on-the-owners-machine-and-its-hosted-cells-under-a-ceiling.md): the multipliers are Linux 1, Linux arm64 1, Windows 2 and macOS 11; of the other 1,000 included minutes, which the account's private repositories share, manual-dispatch Release runs' hosted jobs get at most 600, and every other hosted job of the account shares the rest; a diagnostic run of a cell on the owner's machine runs there.

    > Amended by [ADR 0072](0072-every-feature-in-a-generated-inventory-has-a-passing-test-and-code-coverage-and-mutants-are-signals.md): the ceiling also counts the coverage job and the in-diff mutation job, which stop once the month's count reaches 550 minutes.
  - **Once the nightly tier is on**, the repository is public and its hosted Linux minutes are free: both jobs run in the nightly tier every night, and the ceiling no longer applies.
- **Durations in the store feed the suites' time budgets and the performance trends.** They never fail a test.

### A flaky test found after merge

- **The scan opens one issue per flaky test**, labelled `flaky-test`, in the current milestone, with the runs, seeds and configurations that differed. A recurrence reopens it.
  > Amended by [ADR 0136](0136-each-implement-spec-run-is-one-whole-spec-and-milestones-are-ticket-metadata-and-owner-checkpoints.md): the issue is a sub-issue of the spec that owns the test, in the milestone of that spec's ticket that added it, not in the current milestone.
- **Same day, a pull request adds the test to the test quarantine list with its issue.**
- **The change that brought it may also be reverted** when the test or the code it covers changed in the last 7 days (Mozilla's rule). The revert pull request reverts the product code, and the tests stay as they are, on the quarantine list. The change comes back with the fix.
- **The test quarantine list is one file in the repository**, reviewed like code. Each entry names a test ID, its open `flaky-test` issue and the date. The Quick suite's check refuses an entry with no open issue, or with a test that no longer exists. Each suite command excludes the listed tests through its runner's own filter.
- **A quarantined test keeps running:** the daily test-health job runs it 50 times (the 10 oldest entries) and writes the results to the store, so the issue shows its failure rate and a fix can be confirmed.
- **`flaky-test` issues come first on the frontier**, before every other ticket of the current milestone. Once every V1 milestone is closed, they carry no milestone and still come first.
  > Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): the owner removed this frontier order on 2026-10-18; a `flaky-test` issue is worked like any other issue of its milestone.
- **The fix removes the cause.** A longer timeout alone, a sleep or a retry is not a fix. When the cause is in the product (a race, a leak), the product is fixed and the test stays as it was.
- **It leaves the list only through a fix that passes 200 runs.** The issue closes when that pull request merges.
- **A quarantined test blocks:**
  - its milestone, whose `ready-for-human` gate issue (ADR 0058) is ready only once it is the only open issue left;
  - every release: `xtask release` refuses to cut or promote while the list is not empty;
  - the coverage of the feature it tests, which counts as untested in the check of [Feature coverage and traceability](https://github.com/goodfetch/goodfetch/issues/279).
- **A flaky test is never removed or weakened to make a suite pass:** it is quarantined, fixed, or replaced by a test covering the same behaviour (for example when its unpinned part moves to a Live check and the rest stays a test).

### Live checks

- **Each has an owning area and a declared placement.** The ones decided so far:

| Live check | Tier | What it blocks | Decided in |
|---|---|---|---|
| Real SaaS Sink accounts | nightly | nothing directly | ADRs 0026, 0032 |
| Installed Chrome: the current stable, beta and dev channels | nightly | nothing directly | ADRs 0030, 0032 |
| Public testnets for Paid fetch proofs | nightly | nothing directly | ADR 0045 |
| Clean-up rules' daily channel gate (online Proving ground) | nightly | that night's channel publish | ADR 0047 |
| Online conformance against real vendors | release candidate, Chrome-major and stealth patches | never a release | ADRs 0033, 0043 |
| Benchmark on the reference host | release candidate | the release, by ADR 0025's correctness and Δ rule | ADRs 0025, 0030 |
| Cloud Run deploy | release candidate | nothing directly | ADR 0049 |
| Live-model evaluations | decided by the AI testing ticket | decided by the AI testing ticket | [Testing AI features and agent skills](https://github.com/goodfetch/goodfetch/issues/281) |
| OpenSSF Scorecard | nightly | nothing | ADR 0032 |
| Live advisory audit (`just audit-live`: RustSec, npm and PyPI advisories, newest) | daily on `main`, from the server's cron while the hosted workflows are off | nothing directly | owner decision D6 of 2026-10-22, below |

  "Nothing directly" means only the `live-check` issue rules below apply. A channel moves without a commit, so each channel is a Live check. Each stable major of the last eight weeks and Extended Stable run pinned by version in the nightly matrix, so they are tests; the current stable major is among them, so the channel's Live check only shows when an update within the major breaks something.
  > Amended on 2026-10-22 (owner decision D6, relayed by the orchestrator; agent decision on the details, accepted by the owner on 2026-10-22): the gate's `just audit` read live advisory data, so three advisories published on 10-19 and 10-20 failed `main`'s gate with no commit. It now checks every lockfile offline against a snapshot pinned in `ci/advisories.toml` (RustSec's commit for cargo-deny and cargo-audit; OSV's npm and PyPI databases, by SHA-256, for OSV-Scanner), and the newest data moves to the live advisory audit above, so a new advisory against `main` shows within a day. STYLE.md's "no open security advisory" is unchanged: the gate fails on any advisory in the pinned data.
- **A Live check has three outcomes: pass, fail and inconclusive.** Inconclusive means the external side was unavailable (a vendor outage, an unreachable account, an exhausted quota). It is never counted as a pass.
- **Its results go to the results store** under its own tier, `live`, and show in the same report.
  > Amended by [ADR 0067](0067-security-is-tested-per-trust-boundary-with-secret-canaries-and-private-fuzzing.md): fuzzing's results stay in its private repository, and a crash goes to a private advisory (an issue in that repository before publication) instead of a `live-check` issue, blocking the next minor until it is fixed or closed by triage.
- **A fail, or three inconclusive results in a row, opens or updates one issue per Live check**, labelled `live-check`, for its owning area, in the current milestone. It comes on the frontier right after `flaky-test` issues.
  > Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): the frontier order is removed; a `live-check` issue is worked like any other issue of its milestone.
  > Amended by [ADR 0136](0136-each-implement-spec-run-is-one-whole-spec-and-milestones-are-ticket-metadata-and-owner-checkpoints.md): the issue is a sub-issue of the spec that owns the Live check, in the milestone of that spec's ticket that added it, not in the current milestone.
- **An open `live-check` issue whose cause is not stated within two business days blocks the promotion of the next minor** until the cause is written down: a Goodfetch bug (fixed like any bug) or an external change (recorded). The Kubernetes Informing rule. Patches are not held, and the online conformance keeps ADR 0043's rule that it never blocks.
- **A `live-check` issue closes once its cause is written down and either fixed in Goodfetch or attributed to an external party with a linked upstream report** (the vendor's issue, status-page incident or changelog entry). So an explained vendor outage or testnet change never holds a milestone gate; a later failure reopens the issue.
- **A measurement declares its trial count, threshold and statistic before it runs** (the lab's method), and every trial counts. A failed trial is never replaced by another. Live-model evaluations use pass^k with Wilson intervals; the values belong to [Testing AI features and agent skills](https://github.com/goodfetch/goodfetch/issues/281).

## Considered Options

- **Retries that report the test as flaky but fail anyway** (nextest's `flaky-result = "fail"` with retries on, Playwright's `failOnFlakyTests`) in the gating run: rejected. The verdict is the same, the run costs the extra attempts, and a "flaky" label in a green-looking report trains people to dismiss failures, the complaint Google and Gruber and Fraser record. The diagnostic run gives the same classification in a separate job. The locks stay on, at zero retries.
- **Retrying before merge, as Chromium and Google do**: rejected. Their reason is tens of thousands of existing flaky tests; Goodfetch starts with none, and the owner rules it out.
- **Leaving a flaky test failing until it is fixed (block)**: rejected. It stops every unrelated pull request, which Kubernetes counts as "at least 2 hours of merge time" per recurrence.
- **Quarantine as `STYLE.md` had it, with an issue only**: rejected. With no owner, clock or still-running place, quarantined tests rot; every source that quarantines adds all three.
- **Revert alone, which deletes the test the reverted change added** (Mozilla's backout): rejected. That test is the one that found the flake: deleting it removes a flaky test to make the suite pass and loses its history. Quarantining keeps both.
- **Quarantine held by a service** (Trunk, Datadog, BuildPulse), which turns a red job green after the run: rejected. The verdict then depends on state outside the repository and outside review.
- **A quarantine marker in each test's source** (Kubernetes' `[Flaky]` name tag, Vitest tags, pytest markers): rejected. nextest has no tags, renaming a Rust test breaks its history, and one list gives one check.
- **Datadog Test Optimization for history**: rejected. Its library reports from inside the test process, which ADR 0043's deny blocks, and Rust is not in its library list.
- **Trunk Flaky Tests or Codecov Test Analytics for history**: rejected. Each holds the project's test history in a third-party service, Trunk's free tier stops at five committers, and Codecov only flags. Allure Report 3 (Apache-2.0, release 3.20.1 on 2026-10-06) shows the same history from files the project keeps.
- **ReportPortal**: rejected. It needs a server to run and keep up, for what a static report gives.
- **Keeping results only as Actions artifacts**: rejected. They expire, and nothing compares runs across them.
- **Results on an orphan branch of the main repository**: rejected. Every clone would carry years of results. Deno keeps its benchmark history in a separate repository, `denoland/benchmark_data`.
- **Fewer than 20 stress runs, or a stress run only on request** (Spotify's Flakybot): rejected. 20 is what Chromium and Mozilla settled on; the 200-run bar applies where a known flaky test comes back.
- **A random order seed on every run**: rejected for the merge path. The same commit must give the same order, so a re-run reproduces a failure; the constrained test-health run explores new orders.
- **A daily constrained Pre-merge run while the repository is private**: rejected. A few halved shards a day could take 1,000 to 3,600 minutes a month, past the included minutes, and paid test resources are the owner's to approve. Weekly and after each manual-dispatch Release run, it still catches resource-dependent and order-dependent flakes before the nightly tier exists.
- **Every Live check reports only, Mozilla tier-3 style**: rejected. An unexplained red result would then be ignored; Kubernetes blocks a release until it is explained.

## Consequences

- **Amends `TESTING.md`.** "A flaky test is quarantined with an issue, never retried silently", the line ADR 0062 moved there from `STYLE.md`, becomes "A test is never retried; a flaky test is quarantined the same day, with an issue that comes first and blocks its milestone and every release". The rules are its section Flaky tests and Live checks.
- **Changes area 1's spec** ([Spec: Repository, CI and release (V1 area 1)](https://github.com/goodfetch/goodfetch/issues/130)):
  - user story 65 becomes "a flaky test never retried, quarantined the same day with an issue that blocks releases";
  - in "What makes a good test here", "A flaky test is quarantined with an issue" becomes "A test is never retried, and a flaky one is handled by ADR 0064";
  - the "all green" job also checks earlier attempts; `xtask` gains the no-retry check, the quarantine-list check, the scan and the release refusal; the ingest workflow, the daily test-health job, the constrained test-health run, the diagnostic runs and the `test-results` repository are its jobs.
- **Changes area 3's spec** ([Spec: Engine runtime (V1 area 3)](https://github.com/goodfetch/goodfetch/issues/213)). "flaky tests quarantined with an issue" becomes "flaky tests never retried, and handled by ADR 0064".
- **Amends ADR 0032.** Its rule that the nightly tier is off while the repository is private now excepts the daily test-health job, and the weekly constrained test-health run, within a ceiling of included minutes; a note marks the spot. Once on, the nightly tier hosts both every night. The diagnostic runs follow failures in any tier.
- **Adds to ADR 0030's release gates** two blockers: a non-empty quarantine list blocks every release, and a `live-check` issue whose cause is unstated after two business days blocks the promotion of a minor.
- **Adds to ADR 0058.** `flaky-test` and `live-check` issues sit in the current milestone, so its gate waits for them. A `live-check` issue closes once its cause is written down and fixed or attributed upstream, so only an unexplained or unfixed Live-check failure holds a gate.
  > Amended by [ADR 0136](0136-each-implement-spec-run-is-one-whole-spec-and-milestones-are-ticket-metadata-and-owner-checkpoints.md): they sit in the milestone of the owning spec's ticket that added the test, so that milestone's owner test waits for them; no owner test is a start gate.
- **Adds to ADRs 0025, 0026, 0030, 0033, 0043, 0045, 0047 and 0049.** Where their checks run and what they already block is unchanged. Each of their checks against something unpinned is now a Live check: it reports pass, fail or inconclusive into the results store under the `live` tier, opens a `live-check` issue on a fail or three inconclusive results in a row, and an unexplained failure blocks the next minor after two business days, a new blocking rule for the checks that blocked nothing before. The online conformance keeps ADR 0043's rule that it never blocks. ADR 0037's amendment rule carries this to the areas that build those checks.
- **ADR 0037's table** lists ADR 0064 in area 1's row, which builds its checks, jobs and store.
- **`docs/agents/issue-tracker.md`** puts `flaky-test` issues first on the frontier and `live-check` issues right after, and names both labels.
- **The "Test infrastructure" spec** builds: the `xtask` checks and scan, the stress selection per runner, the order seed, the ingest workflow, the `test-results` repository and its token, the daily test-health job, the Allure build, the diagnostic jobs, the quarantine-list format and its exclusion per runner, the minute ceiling's count and skips, and the two labels.

> Corrected on 2026-10-19 in [#370](https://github.com/goodfetch/goodfetch/issues/370): the live-model rows of the Live check table and the live-model measurement rule, left to [#281](https://github.com/goodfetch/goodfetch/issues/281), are decided since in [ADR 0065](0065-ai-features-are-tested-without-live-models-and-measured-by-oracle-graded-evaluations.md), which lists the live-model checks; `TESTING.md` says the Live checks are listed in ADRs 0064 and 0065.

> Corrected on 2026-10-19 in [#370](https://github.com/goodfetch/goodfetch/issues/370): the 2,000 included minutes are GitHub Free's, and every ceiling here assumes the account stays on it: GitHub Pro bills, so it is declined (ADR 0076).
