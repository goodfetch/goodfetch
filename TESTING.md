# Testing

How Goodfetch is tested: every surface, at every level. Each rule comes with its reason, and the hard-to-reverse ones are decided in ADRs. [`STYLE.md`](STYLE.md) shapes the code these tests drive. Here a *snapshot* is a stored expected value, never a Checkpoint or a Group backup.

## Contents

1. [Levels and layout](#levels-and-layout)
2. [Writing a test](#writing-a-test)
3. [Dependencies in tests](#dependencies-in-tests)
4. [Shared helpers](#shared-helpers)
5. [Fixtures and data files](#fixtures-and-data-files)
6. [Snapshots](#snapshots)
7. [Property tests, fuzzing and doctests](#property-tests-fuzzing-and-doctests)
8. [Injection and determinism](#injection-and-determinism)
9. [Flaky tests and Live checks](#flaky-tests-and-live-checks)
10. [Test-first process](#test-first-process)
11. [Review checklist](#review-checklist)

Sections 1 to 7, 10 and 11 are decided in [ADR 0062](docs/adr/0062-tests-sit-beside-their-code-and-share-one-test-support-written-once.md).

## Levels and layout

A test's level is the seam it crosses, not its speed:

- A **unit test** tests one module through its interface, in one process.
- An **integration test** tests one package through its public interface, with real local stand-ins and real processes.
- An **end-to-end test** follows a user journey through a shipped surface.
- The **conformance suites** of the ADRs stay the interface tests of every contract with several implementations.

| | Unit tests | Integration tests |
|---|---|---|
| Rust crate | `#[cfg(test)] mod tests` at the end of the module file, or in `<module>/tests.rs` | `tests/it/main.rs` and its modules |
| TypeScript package, Console | `foo.test.ts` (or `.tsx`) beside `foo.ts` | `<package>/tests/`, importing only the package's own name |
| Python package | `tests/unit/`, mirroring the module paths under `src/` | `tests/integration/`, importing only the public package |

- **A unit test reaches only its module's interface:** the items the module's callers use. Never a private function.
  _Why:_ such tests survive internal refactors, and they test what callers rely on.
- **Product code has no `#[cfg(test)]` branch and no `pub` item added for tests.** If a test needs one, the module is the wrong shape.
- **One integration-test binary per crate, run with `cargo-nextest`.** A crate splits it into named area harnesses (`tests/<area>/main.rs`, `autotests = false`) only when its pull request shows a localized test edit taking more than 5 seconds to rebuild with one harness.
  _Why:_ each file in `tests/` links its own binary. uv's measurement favours one harness until a suite is large.
- **Tests of the `goodfetch` binary live in its crate's `tests/it`** and find the binary through `CARGO_BIN_EXE_goodfetch`, never a guessed path.
  _Why:_ the build directory's layout may change without notice.
- **Python tests run with `--import-mode=importlib` and `strict = true`** in `[tool.pytest]`.
  _Why:_ tests outside the package run against the installed wheel, as pytest recommends.

## Writing a test

- **One behaviour per test, through the interface.** Several assertions are fine when they check that one behaviour.
- **Arrange, act, assert, in that order, with one act.**
- **No branch or loop computes an expected value.** Expected values come from a literal, the spec or the Proving ground's oracle.
  _Why:_ an expected value computed the way the code computes it can never disagree with the code.
- **Assert a failure on its Error code, never on its message text.**
  _Why:_ Error codes are the contract (ADR 0038); messages change freely.
- **Every test creates its own data.** No test reads another test's data or depends on test order.
- **Names state the behaviour in glossary terms**, never `_Avoid_` terms:
  - Rust: `resumed_run_never_repeats_an_effect`, with no `test_` prefix.
  - Python: `test_resumed_run_never_repeats_an_effect`.
  - TypeScript: `describe("<interface under test>")` with `test("resumed run never repeats an Effect")`, at most two `describe` levels.
  - No `should`, no issue numbers, no `it_works`.
  - Integration-test modules and files are named by behaviour area (`tests/it/resume.rs`), not by source file.
- **Every case of a parameterized test has a name:** rstest's `#[case::name]` in Rust, Vitest's `test.for` with the name in the title, pytest's `parametrize` with an `id` per case. Never loop over cases inside one test.
  _Why:_ a loop stops at the first failing case and hides the others; rstest makes each case its own nextest test.
- **No mocking of owned code:**
  - `cargo-deny` denies `mockall`, `mockito`, `wiremock`, `faux` and `mry`;
  - oxlint's `vitest/no-restricted-vi-methods` refuses at least `vi.mock`, `vi.doMock`, `vi.importMock`, `vi.mockObject`, `vi.spyOn`, `vi.stubGlobal` and `vi.stubEnv`;
  - ruff's `TID251` bans `unittest.mock` and `pytest.MonkeyPatch`;
  - pytest-mock is in no dependency group (CI checks `uv.lock`), and the test-support pytest plugin fails the collection of any test that requests `monkeypatch` or `mocker`, directly or through another fixture. A third-party plugin fixture that uses `monkeypatch` therefore cannot be adopted.

  _Why:_ a mocked collaborator ties the test to the implementation.
- **The TypeScript and Python SDKs read their environment from a map an option can replace**, defaulting to the process's own. Tests pass the map instead of patching the environment.
  _Why:_ a patched environment leaks between tests and hides what a call depends on.
- **A test is never retried.** A flaky test is quarantined the same day, with an issue that comes first and blocks its milestone and every release ([Flaky tests and Live checks](#flaky-tests-and-live-checks)).
  _Why:_ a retry turns a real race into a green run, and people stop trusting red ones.

## Dependencies in tests

A module is tested according to the kind of dependency it has, using the four categories of `codebase-design`:

- **In-process** (pure logic: URL normalization, robots, frontier rules, extraction, run-state rules): through the module's interface; `proptest` for parsers.
- **Local-substitutable** (a real local stand-in exists): SQLite in a temporary file; a real Chromium (the Goodfetch build of ADR 0035) against Proving ground pages served locally. CDP is never mocked.
  _Why:_ a fake browser lets tests pass where a real one fails, which is the false success ADR 0025 measures.
- **Remote but owned** (Goodfetch's own processes across a boundary: sink processes, process Plugins, the Daemon): the logic is tested through an in-memory adapter of the protocol, and the real processes through the conformance suites.
- **True external** (model providers, CAPTCHA services): a mock adapter replaying recorded exchanges. httpmock is the HTTP mock for this.

How each dependency is injected is in [Injection and determinism](#injection-and-determinism).

## Shared helpers

- **Rust helpers live in `goodfetch-test-support`** (`crates/goodfetch-test-support`, `publish = false`): the per-test folder, `GOODFETCH_HOME` and cleared environment, the printed seed, unique names, reported addresses, the Proving ground started from a seed, commands with the cleared environment, and the shared snapshot redactions and filters.
- **`goodfetch-test-support` depends on no Goodfetch crate**, and product crates list it only under `[dev-dependencies]`. CI checks both.
  _Why:_ a helper crate that depended on the crate under test would give that crate's tests two copies of its types; values are all ADR 0059 injects.
- **The test versions of `Clock` and `Entropy` are `Clock::paused(start)` and `Entropy::seeded(seed)`**, in the crate that defines the handles. `disallowed-methods` refuses both; each test module or integration-test root that calls them opts in with `#[expect(clippy::disallowed_methods, reason = "…")]` (`#![expect]` in `tests/it/main.rs`), never with a crate-wide or `cfg_attr(test, …)` attribute in product code. `main` builds only the system versions. Tests that pause Tokio's clock enable its `test-util` feature under `[dev-dependencies]`. The seed and the start time come from `goodfetch-test-support`.
- **TypeScript and Python reuse the Rust setup, never rewrite it.** The `goodfetch-test-env` executable of `goodfetch-test-support` prepares a test's folder, home and environment, starts what it is asked to, prints one JSON description (paths, environment map, seed, addresses), and tears everything down when its standard input closes.
  - `@goodfetch/test-support` (`test-support/typescript/`, private) gives Vitest fixtures through `test.extend`.
  - `goodfetch-test-support` (`test-support/python/`, never published) is a pytest plugin registered through `pytest11`.
- **`goodfetch`'s `test-util` feature is for people who embed `goodfetch`**, never Goodfetch's own helper home.
- **Expected results of Proving ground pages come from its oracle**, never copied into test code.
- **A helper written twice moves into test support.**

## Fixtures and data files

- **Fixtures live in a `fixtures/` folder beside the tests that read them.** Data several languages read lives once, in `test-support/fixtures/`.
- **A test copies a fixture into its temporary folder before changing it.**
- **No fixture file exceeds 256 KiB.** Larger or repetitive data is generated from a seed, by the Proving ground or a generator in test support. No Git LFS.
- **A generated fixture or a recorded exchange has a `<name>.source.toml` file beside it,** naming the generator or recording job, its version, the seed or recording date, and the command that regenerates it. Secrets are redacted from recordings.
  _Why:_ a fixture nobody can regenerate becomes untouchable.

## Snapshots

- **Tools:** insta in Rust, Vitest's snapshots in TypeScript, inline-snapshot in Python.
- **Inline for short values** (about 20 lines); a file for longer ones: insta's `.snap` files, Vitest's `toMatchFileSnapshot` or `toMatchSnapshot`, inline-snapshot's `external()`.
- **What snapshots cover:** structured outputs (JSON-RPC responses, CLI output, generated surfaces, extraction output). Never instead of a precise assertion on one value.
- **Every varying value is redacted or filtered** (IDs, times, durations, temporary paths), with the shared settings of test support.
- **CI never writes a snapshot and fails on a mismatched, missing or orphaned one:** `cargo insta test --check --unreferenced reject`; Vitest in CI; inline-snapshot, which raises on an empty `snapshot()` there.
- **Snapshots change only through the tool's update or review command, never by hand.** inline-snapshot updates run single-process (`-n0`), since under xdist it can neither create nor fix one.
- **A snapshot change is reviewed as a change of expected behaviour.**

## Property tests, fuzzing and doctests

- **Parsers and stateful rules get property tests:** `proptest` in Rust, `hypothesis` in Python. Their regression files are committed.
- **Untrusted input** gets a `cargo-fuzz` target per decoder: CDP messages, JSON-RPC and plugin-protocol framing, HTML and Markdown, sitemaps, run-state import.
- **Doctests only on `goodfetch`**, where they are the examples of ADR 0030.

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
- **Tests run in random order, seed printed:** pytest-randomly, Vitest `sequence.shuffle.tests`, Playwright `--shuffle` with `fullyParallel`; nextest runs each test in its own process. The seed comes from the tree hash under test in every run but the constrained test-health run, which takes a fresh one. Pass it back to reproduce an order.
- **Vitest resets shared state:** `restoreMocks`, `unstubEnvs` and `unstubGlobals` are on.
- **Leaks and hangs fail:** nextest's leaky result is `fail`, every test has its suite's hang timeout, and the constrained test-health run runs Vitest with `detectAsyncLeaks`.

### After merge

- **Every runner writes JUnit XML in every tier and attempt, pass or fail.** An ingest workflow, which runs none of the product, copies it to the results store: the `test-results` repository beside the main one (private until it is public), one compressed file per run attempt, with a record per test run (test ID, runner, suite, tier, job, OS, architecture, runtime, shard, commit, tree hash, run, attempt, outcome, duration, order and `Entropy` seeds). Nothing is pruned.
- **Allure Report 3 shows the history,** built from the store by the daily test-health job.
- **A test is flaky when** it passed and failed on the same tree hash and configuration, or when a diagnostic run passes it at least once: 20 more runs at the same commit and on the same OS, after a failure in the merge queue, a manual-dispatch Release run, the nightly tier or a test-health run. A diagnostic run never changes a verdict.
- **Test health runs from M1 on, even while the repository is private, with no paid resource:**
  - the **daily test-health job** (scheduled, Linux, `main`'s head, 15-minute timeout) runs the 10 oldest quarantined tests 50 times each, then the scan and the Allure report;
  - the **constrained test-health run** (weekly and after each manual-dispatch Release run, Linux, 120-minute timeout) runs the Pre-merge suite with half the runner's CPU and memory, a fresh order seed and `detectAsyncLeaks`;
  - these and the diagnostic runs share a ceiling of 1,000 included minutes a month. At the ceiling, constrained and then diagnostic runs are skipped until the month ends.
  - Once the nightly tier is on, both run every night with no ceiling.

### A flaky test found after merge

1. CI opens (or reopens) one `flaky-test` issue for it, in the current milestone.
2. The same day, add the test to the quarantine list with its issue.
3. If the test or the code it covers changed in the last 7 days, you may also revert that change: revert the product code, and the tests stay quarantined.
4. `flaky-test` issues come first on the frontier.
5. Fix the cause. A longer timeout alone, a sleep or a retry is not a fix. A race or leak in the product is fixed in the product, and the test stays as it was.
6. The fix takes the test off the list after 200 passing runs; the issue closes when it merges.

- **The quarantine list is one file in the repository.** Each entry has a test ID, its open `flaky-test` issue and a date; the Quick suite's check refuses an entry without an open issue or naming a test that no longer exists.
- **Quarantined tests still run** 50 times a day in the daily test-health job (the 10 oldest entries), into the results store.
- **A quarantined test blocks** its milestone's gate, every release (`xtask release` refuses while the list is not empty), and counts its feature as untested.
- **A flaky test is never removed or weakened to make a suite pass:** quarantine it, fix it, or replace it with a test covering the same behaviour (for example by moving its unpinned part to a Live check).

### Live checks

- **Each Live check has an owning area and a placement** listed in ADR 0064: real SaaS Sinks, installed Chrome's stable, beta and dev channels (pinned majors are tests), public testnets, the Clean-up rules' daily gate, the online conformance, the benchmark, the Cloud Run deploy, live-model evaluations, Scorecard.
- **Outcomes are pass, fail or inconclusive.** Inconclusive means the external side was unavailable; it never counts as a pass.
- **Results go to the results store** under the `live` tier.
- **A fail, or three inconclusive runs in a row, opens or updates one `live-check` issue** in the current milestone, right after `flaky-test` issues on the frontier. If its cause is not stated within two business days, the next minor is not promoted until it is. Patches are not held, and the online conformance never blocks (ADR 0043).
- **A `live-check` issue closes** once its cause is written down and either fixed or attributed to an external party with a linked upstream report, so an explained vendor outage never holds a milestone gate.
- **A measurement declares its trials, threshold and statistic before it runs,** and every trial counts. A failed trial is never replaced.

## Test-first process

- **An agent implementing a ticket follows the `tdd` skill.** The seams named in the spec's Testing Decisions are the seams agreed in advance. A test at another seam needs the pull request to name that seam and the reason, and the fresh review to accept it.
- **Every behaviour lands red, then green.** The pull request's Verification section lists each new test with the failure it showed before the code existed. Commits stay green.
  _Why:_ a test never seen failing may test nothing, and green commits keep `git bisect` and the "all green" check working.
- **A bug fix starts with a test that reproduces the bug**, named by the behaviour, not the issue.
- **Refactoring is not part of the red-green loop;** it happens in review.

## Review checklist

The fresh review of a pull request checks its tests against this list:

1. Each test sits at a seam the spec names, or the pull request justifies the new seam.
2. The Verification section lists each new test with its red failure.
3. One behaviour per test, named as a behaviour in glossary terms, in the right place for its level.
4. Through the interface: no private call, no `#[cfg(test)]` in product code, no `pub` added for tests, no mock of owned code.
5. Expected values come from an independent source; failures are matched on Error codes.
6. Deterministic and isolated: the rules of [Injection and determinism](#injection-and-determinism), no real sleep, its own data.
7. Parameterized cases are named.
8. Snapshots are redacted, cover structured output, and every change to one is intended.
9. Fixtures are under 256 KiB, and generated ones have their `.source.toml` file.
10. No helper is duplicated; shared setup is in test support.
