# Testing

How Goodfetch is tested: every surface, at every level. Each rule comes with its reason, and the hard-to-reverse ones are decided in ADRs. [`STYLE.md`](STYLE.md) shapes the code these tests drive.

## Injection and determinism

Decided in [ADR 0059](docs/adr/0059-dependencies-are-injected-as-values-and-only-time-and-randomness-as-handles.md).

### What is injected, and how

| Dependency | Injected as | In tests |
|---|---|---|
| Wall time, monotonic time, sleeps | the `Clock` handle | its test version under Tokio's paused clock |
| Random choices (jitter, pool picks, fingerprint seeds, ID randomness) | the `Entropy` handle | a fixed seed, printed at test start |
| Secret randomness (keys, tokens, run data keys, CAs) | nothing: the OS generator | the OS generator |
| Data, caches and configuration | the `GOODFETCH_HOME` value | a per-test temporary folder |
| SQLite and files | paths under that home | real files |
| Listen addresses | a value (port 0, or a socket path) | the bound address reported back |
| Host names | the `resolve` value | Proving ground hosts mapped to loopback |
| Environment | an explicit map | a cleared map for each child process |
| Keychain, service manager, processes | nothing: the real service | the real service, unique names |

- **A dependency with a local stand-in gets no handle.** Only `Clock` and `Entropy` are swapped in tests.
  _Why:_ a fake passes where the real thing fails, and a stand-in tests what users run.
- **Nothing reads time, chooses at random or reads the environment through a global.** `disallowed-methods` refuses `SystemTime::now`, std and Tokio `Instant::now` and constructors such as `Ulid::new` outside `Clock` and `Entropy`, and `std::env::var` outside `main`; `std::env::set_var` is `unsafe`.

### Time

- **A rule that depends on time is a pure function that takes "now"**, tested with no clock: lease expiry, a Schedule's next occurrence (DST included), backoff. The Politeness store's granted shares stay in its Lua script and are tested against a real Valkey.
- **Code that waits is tested in-process with the test `Clock`** (`#[tokio::test(start_paused = true)]`). Advancing the paused clock moves wall time with it, and `jump_wall` moves wall time alone.
  _Why:_ a default-length lease or a day-long interval then takes no real time.
- **Keep real I/O out of paused-clock tests**, apart from SQLite on tracked blocking threads.
  _Why:_ auto-advance fires whenever the runtime is idle, including during a real socket wait, so a lease can expire early (tokio #4522, #8232).
- **A test of the binary sets short durations through settings.** No build has a test-only clock control.
- **Never sleep for real** beyond a short duration the test set itself.
- **Clocks Goodfetch doesn't own stay real:** the Valkey server's (short durations), Chromium's (Proving ground pages take their time step from the task), and the Script sandbox's live `Date.now`, whose tests compare the order of Effects.

### Randomness

- **Every random choice draws from `Entropy`** (`chacha20`, portable across versions).
- **A test runs on a fixed seed.** `GOODFETCH_TEST_SEED` overrides it, and the seed is printed when the test starts.
  _Why:_ a test nextest kills on a timeout never reaches a failure message.
- **Secrets never come from `Entropy`.** The Proving ground's seeded test certificate authority is the one exception.
- **Hash order never reaches output or ordering.** `iter_over_hash_type` is denied; use `BTreeMap` or `IndexMap` where order shows.

### Storage, network and OS services

- **Each test gets its own `GOODFETCH_HOME`** in a temporary folder, and writes nowhere else.
- **Proving ground host names resolve through the `resolve` setting** to loopback, which the run allowlists for tests.
- **Proxies are tested against the Proving ground's real forward proxy.**
- **Local addresses use real extra loopback addresses:** `127.0.0.0/8` on Linux and Windows (Windows not yet tested), `lo0` aliases on macOS, a network namespace under sudo where needed.
- **No test reaches the public internet.** Only the jobs ADR 0043 names go online, to their named endpoints; every other job runs under its deny with the canary.
- **The keychain provider runs against the real keychain** of each OS (`gnome-keyring` started and unlocked on Ubuntu), with a unique service name per test. Consumers of Secrets use the environment or caller provider.
- **`daemon install` installs for real on each OS.** The unit file it generates is snapshot-tested.

### Isolation

1. One process per test (nextest).
2. A per-test temporary `GOODFETCH_HOME`.
3. A child process starts with a cleared environment whose `HOME`, `USERPROFILE` and `XDG_` variables point inside the test's folder.
4. No fixed ports: bind port 0 and report the address, or use a Unix socket or named pipe in the test's folder.
5. One Valkey per CI job, with a namespace per test.
6. Resources shared by the whole user (keychain, service manager) run in a nextest test group, with unique names.
7. Certificate authorities are made per test: the Proving ground's from its seed, a Daemon group's by `goodfetch group init` into the test's folder.
8. No test reads another test's data or depends on test order.

### SDKs and the Console

- **SDK durations and jitter are options:** tests pass short values and a fixed seed. In TypeScript, Vitest fake timers may drive timer logic that never reaches the engine.
- **The Console's end-to-end tests set time with Playwright's `page.clock`** (`setFixedTime` first). Its component tests use Vitest fake timers.

## Flaky tests and Live checks

Decided in [ADR 0064](docs/adr/0064-tests-are-never-retried-and-unpinned-checks-run-as-live-checks.md). The Quick, Pre-merge and Release suites and the CI tiers are those of [Test suites, local commands and time budgets](https://github.com/Jbmanllr/automan/issues/276).

### What is a test

- **A test pins every input:** commit, Proving ground seed, `Entropy` seed, browser build, images by digest, recorded model exchanges. A **flaky test** passes and fails with the same inputs.
- **A check with an input that moves without a commit is a Live check.** It never goes in a suite.
  _Why:_ a vendor's change or outage is then never mistaken for a broken change.

### No retries

- **Runners keep zero retries with every lock on:** nextest `retries = 0` and `flaky-result = "fail"` in every profile, with no per-test override; Vitest `retry: 0` with no per-test `retry`; Playwright `retries: 0` and `failOnFlakyTests: true`; no pytest rerun plugin and no `flaky` marker.
- **No workflow step retries a test:** no retry action, no shell loop, no `continue-on-error` on a test step.
- **No test loops until it passes.** Wait on a condition with a deadline (auto-waiting, `expect.poll`, a CDP event, a state you can name); never repeat the action under test until it succeeds.
- **The no-retry check in the Quick suite refuses all of the above**, and the flags and variables that undo them (nextest `--retries`, `--flaky-result`, `NEXTEST_RETRIES`, `NEXTEST_FLAKY_RESULT`; Vitest `--retry`; Playwright `--retries`). Downloads before the deny of ADR 0043 may retry: they test nothing.
- **Re-run a failed job only when no test step failed in it** (lost runner, failed download). The "all green" job fails when an earlier attempt of its run had a failed test step.
- **Never assert elapsed real time, a rate or a duration.** Those are measurements.

### Before merge

- **Every new or changed test runs 20 times in the pull request**, and one failure fails it. New: absent from the base commit's listing. Changed: its file changed. The run is sharded to fit the budget; the count is never lowered, and nothing skips it.
- **Taking a test off the quarantine list takes 200 passing runs** in that pull request.
- **Tests run in random order, seed printed:** pytest-randomly, Vitest `sequence.shuffle.tests`, Playwright `--shuffle` with `fullyParallel`; nextest runs each test in its own process. The seed comes from the commit's tree hash on pull requests, the merge queue and `main`, and is fresh in the daily test-health job. Pass it back to reproduce an order.
- **Vitest resets shared state:** `restoreMocks`, `unstubEnvs` and `unstubGlobals` are on.
- **Leaks and hangs fail:** nextest's leaky result is `fail`, every test has its suite's hang timeout, and the daily test-health job runs Vitest with `detectAsyncLeaks`.

### After merge

- **Every runner writes JUnit XML in every tier and attempt, pass or fail.** An ingest workflow, which runs none of the product, copies it to the results store: the `test-results` repository beside the main one (private until it is public), one compressed file per run attempt, with a record per test run (test ID, runner, suite, tier, job, OS, architecture, runtime, shard, commit, tree hash, run, attempt, outcome, duration, order and `Entropy` seeds). Nothing is pruned.
- **Allure Report 3 shows the history,** built from the store by the daily test-health job.
- **A test is flaky when** it passed and failed on the same tree hash and configuration, or when a diagnostic run (20 more runs at the same commit, after a failure in the merge queue, on `main`, nightly or in the daily test-health job) passes it at least once. A diagnostic run never changes a verdict.
- **The daily test-health job runs from M1 on, even while the repository is private** (one scheduled Linux job on `main`): the Pre-merge suite with half the runner's CPU and memory, a fresh order seed and `detectAsyncLeaks`; then each quarantined test 50 times; then the Allure report.

### A flaky test found after merge

1. CI opens (or reopens) one `flaky-test` issue for it, in the current milestone.
2. The same day, add the test to the quarantine list with its issue. The test file stays.
3. If the test or the code it covers changed in the last 7 days, you may also revert that change: revert the product code and keep every test file. A test the change added stays and goes on the quarantine list; a test it changed keeps its new content.
4. `flaky-test` issues come first on the frontier.
5. Fix the cause. A longer timeout alone, a sleep or a retry is not a fix. A race or leak in the product is fixed in the product, and the test stays as it was.
6. The fix takes the test off the list after 200 passing runs; the issue closes when it merges.

- **The quarantine list is one file in the repository.** Each entry has a test ID, its open `flaky-test` issue and a date; the Quick suite's check refuses an entry without an open issue or naming a test that no longer exists.
- **Quarantined tests still run** 50 times a day in the test-health job, into the results store.
- **A quarantined test blocks** its milestone's gate, every release (`xtask release` refuses while the list is not empty), and counts its feature as untested.
- **No agent removes a test or a test file**, by a revert or otherwise. Fix a flaky test, or move its unpinned part to a Live check. Only the owner removes a test (for example one whose behaviour left the product): propose the exact paths and commands.

### Live checks

- **Each Live check has an owning area and a placement** listed in ADR 0064: real SaaS Sinks, installed Chrome's stable, beta and dev channels (pinned majors are tests), public testnets, the Clean-up rules' daily gate, the online conformance, the benchmark, the Cloud Run deploy, live-model evaluations, Scorecard.
- **Outcomes are pass, fail or inconclusive.** Inconclusive means the external side was unavailable; it never counts as a pass.
- **Results go to the results store** under the `live` tier.
- **A fail, or three inconclusive runs in a row, opens or updates one `live-check` issue** in the current milestone, right after `flaky-test` issues on the frontier. If its cause is not stated within two business days, the next minor is not promoted until it is. Patches are not held, and the online conformance never blocks (ADR 0043).
- **A measurement declares its trials, threshold and statistic before it runs,** and every trial counts. A failed trial is never replaced.
