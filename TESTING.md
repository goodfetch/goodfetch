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
9. [Simulation, model checking and crash testing](#simulation-model-checking-and-crash-testing)
10. [Flaky tests and Live checks](#flaky-tests-and-live-checks)
11. [Test-first process](#test-first-process)
12. [Review checklist](#review-checklist)
13. [Suites, commands and time budgets](#suites-commands-and-time-budgets)
14. [AI features and agent skills](#ai-features-and-agent-skills)
15. [Journeys](#journeys)
16. [Upgrades and version mismatches](#upgrades-and-version-mismatches)
17. [Security](#security)

Sections 1 to 7, 11 and 12 are decided in [ADR 0062](docs/adr/0062-tests-sit-beside-their-code-and-share-one-test-support-written-once.md).

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
- **Product code has no `#[cfg(test)]` branch and no `pub` item added for tests.** If a test needs one, the module is the wrong shape. The one test-only `cfg` in product code is the `sync` modules' `goodfetch_loom` switch ([Concurrency](#concurrency)).
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
- **Remote but owned** (Goodfetch's own processes across a boundary: sink processes, process Plugins, the Daemon): the logic is tested through an in-memory adapter of the protocol and, for several hosts, in the [Simulation](#simulation), and the real processes through the conformance suites.
- **True external** (model providers, CAPTCHA services): a stand-in adapter. For Model providers, the Scripted model in-process, Model recordings replayed by `httpmock` at the adapter's base URL (matched on the body, secrets removed by an allowlist), and the Proving ground's Fake model end to end; no test calls a live model ([AI features and agent skills](#ai-features-and-agent-skills), ADR 0065).

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
- **Untrusted input** gets a `cargo-fuzz` target for every decoder of input Goodfetch did not write itself, replayed by a plain `#[test]` on every OS. The list and the rest of fuzzing are under [Security](#security) (ADR 0067).
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

- **A dependency with a local stand-in gets no handle.** Only `Clock` and `Entropy` are swapped in tests, and, in the [Simulation](#simulation) alone, `transport`'s network (ADR 0060). That network is a value with product adapters, the OS and the in-memory pair the embeddable `goodfetch` crate uses, so its interface is public for them, not for tests, and it is not a third handle.
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
- **No test reaches the public internet.** Only the jobs ADR 0043 names go online, to their named endpoints; every other job runs under its deny with its Egress canary.
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

## Simulation, model checking and crash testing

Decided in [ADR 0060](docs/adr/0060-distributed-and-crash-behaviour-is-simulated-on-tokio-model-checked-and-faulted-at-io-seams.md). Leases, run epochs, Checkpoints, the writer lock, Daemon groups, Partitioned crawls, degraded shares, the Usage ledger and Sink delivery are tested at every layer that can break them; the ADR's table says which.

### Code the tests rely on

- **Open the connections the Simulation carries only in `goodfetch-engine`'s `transport` module**, which takes the network as a value (the OS, the in-memory pair, or a test adapter): stdio of an Attached engine, the Daemon's socket and named pipe, loopback TCP, the WebSocket listener, the private endpoint, the ops listener, the peer link, wake endpoints. `rpc` takes its connections from it. Fetch sockets, the CDP connection, plugin-protocol pipes and third-party clients stay outside the Simulation and open their own. `STYLE.md` lists the lints.
  _Why:_ the Simulation swaps the network there, and nowhere else.
- **Start threads and block only through the `tasks` module**, whose blocking policy is a value. In the Simulation it runs blocking calls inline, and a dedicated thread's body (the run's SQLite writer actor, for example) as a task on the host's runtime.
  _Why:_ a real thread ends its work at an arbitrary simulated moment, so the run would not replay.
- **Never build a runtime in the engine.** Only `main` and test helpers build one; `tokio::task::block_in_place` is in `disallowed-methods`.
- **Give every third-party time hook the `Clock`** (rustls's `TimeProvider`), and the test `tracing` subscriber's timer too, so traces carry simulated time.
- **Emit each transition a TLA+ model names as a `tracing` event** under the model's target, with the model's variable names as fields.
- **No failpoints.** Inject faults at I/O seams: the crash VFS, the transport adapters, the plugin protocol's in-memory adapter, Toxiproxy, process kills.
  _Why:_ a crash only matters at an I/O boundary, and each boundary already has a seam.

### Simulation

- **Several hosts in one process run on turmoil**, pinned, each on its own paused current-thread Tokio runtime with `Clock::paused` and `Entropy::seeded`, built with `--cfg tokio_unstable` so each host's runtime randomness (`select!` order) follows the seed. No crate swaps the runtime.
- **Compile the scenarios only under `cfg(tokio_unstable)`** (`#[cfg(tokio_unstable)] mod simulation;` in `tests/it/main.rs`). They run from a nextest archive and CI job of their own, filtered to the `simulation` module, with their own rust-cache key, counted against the Pre-merge budget and ADR 0061's cache priority.
  _Why:_ every other test then runs on the Tokio configuration production ships.
- **Derive every seed from the test's seed**, which `goodfetch-test-support` supplies (`GOODFETCH_TEST_SEED` overrides it): one stream for turmoil, one per host's `Entropy`, one for the Fault plan's generation.
- **A scenario keeps an Observation log** of operations, answers and faults, and **the shared invariant checkers judge it**: fencing, one lead, Limits, Partitions, the Audit log chain (through the product's own verifier, passed in as a value), and liveness within a stated simulated time once faults stop. Real-process tests and conformance suites use the same checkers.
- **Faults come from Fault plans** generated by proptest-state-machine and shrunk by proptest on failure.
- **Check determinism:** rerun a few seeds and compare Observation logs and `tracing` output. Fix any mismatch at its source.
- **Leave security values out of the comparison.** Keys, tokens, certificates and TLS handshakes still come from the OS generator (ADR 0059) and are recorded redacted. If behaviour depends on their bytes, fix the code; never seed them.
- **The Politeness store is not simulated.** Group scenarios set `politeness_sharing: per_daemon`.

### Model checking

- **The five protocols have TLA+ models checked by TLC:** run fencing, lead epoch, Partition handoff, slices and reclaim, degraded shares. The TLA+ tools and the CommunityModules release are pinned as one pair, which the "Test infrastructure" spec chooses (whether stable 1.7.4 runs trace validation is unverified).
- **Trace validation:** TLC checks the Simulation's NDJSON traces against the model. Real-process traces are not validated, since several processes' events have no single order.
- **Change a model in the same pull request as the behaviour it describes.**

### Model-based tests and crashes in one process

- **Test `run_state`, `lease`, `checkpoint`, delivery and `ledger` with proptest-state-machine** against reference models, on real SQLite in the test's folder with `Clock::paused`, with crash, power-cut, timeout, late-commit and resume transitions.
- **Crash SQLite through the crash VFS** (`sqlite-plugin`): it holds writes until a sync; a process crash keeps unsynced writes, a power cut drops, tears or reorders them from the seed; the crashed instance's later I/O is refused. It also injects I/O errors and a full disk. Databases open with the VFS name as a value; production passes none.
- **Check after every crash:** acknowledged commits are present in default durability, `PRAGMA integrity_check` passes in both modes, and nothing is visible twice.
- **Test Sinks through the plugin protocol's in-memory adapter** with a test Sink of each Guarantee class, and judge the result by class.

### Concurrency

- **A module that synchronizes threads with its own atomics, `Mutex`, `RwLock` or `Condvar` has loom tests.** It imports them through its crate's `sync` module, which maps to loom under `--cfg goodfetch_loom`.
  _Why:_ a Goodfetch-specific cfg leaves dependencies that react to `cfg(loom)` unchanged.
- **No Shuttle.** Actors on Tokio channels are covered by the Simulation.

### Real processes

- **Kill real processes at seeded moments** chosen from observed protocol events: `kill -9` on Linux and macOS, `TerminateProcess` on Windows, for engines, Clients, sink processes and Script sandbox workers.
- **Power-cut the real binary with LazyFS on Linux:** mount the run folder and the Goodfetch home through it, kill, clear its cache, resume, check as above.
- **Assert `fullfsync` on macOS**, which no tool can observe reaching the drive.
- **Cut off engines and partition Group members with Toxiproxy**, reached through each engine's `resolve` mapping. Give each engine's listeners a loopback address of their own (`resolve` maps names, not ports). Keep Valkey's durations short.
- **Run ADR 0055's Kubernetes path on `kind`.**

### Where the pieces live

- **Simulation scenarios:** `goodfetch-engine`'s `tests/it/simulation/`, one module per behaviour area, with turmoil's network adapter.
- **Committed Fault plans:** `fixtures/fault_plans/` beside their scenario, each with its `<name>.source.toml` (generator version, seed, replay command).
- **Model-based tests and loom tests:** unit tests of the module they check; loom tests compile only under `goodfetch_loom`.
- **The crash VFS, the Observation log and the invariant checkers:** `goodfetch-test-support`. The Observation log is plain records, so they need no Goodfetch crate.
- **TLA+ models:** `test-support/models/`, one folder per model with its TLC configurations and trace-validation spec.

### Suites

Placed in the suites of [Suites, commands and time budgets](#suites-commands-and-time-budgets). Nightly-only jobs explore new seeds: they repeat tests of normal length with new `GOODFETCH_TEST_SEED` values, each within the 5-minute kill, in the nightly tier once it is on. They are in no suite. A failure in the deterministic layers (model-based tests, Simulation, loom, TLC) replays from its seed; a real-process or Toxiproxy failure is recorded with its seed and classified by ADR 0064's diagnostic run.

| Check | Quick | Pre-merge | Release | Nightly-only job |
|---|---|---|---|---|
| Pure time rules, proptest of decision logic | yes | yes | yes | |
| Model-based tests with the crash VFS | 16 cases and committed regressions, within the 10 s kill | own job: 256 cases and committed regressions | as Pre-merge, every OS | random seeds, 1 h |
| Simulation (own `tokio_unstable` job) | | committed Fault plans, 16 seeds per scenario, determinism check on 4 seeds, within 10 min | as Pre-merge | random seeds, 2 h |
| loom | | changed crates, 2 preemptions, within 10 min | every crate, 2 preemptions | every crate, 3 preemptions |
| TLC | | when a model or traced module changes, with trace validation, within 10 min | every model | larger constants, 2 h |
| Real-process crashes, writer lock, shared per-user files | | fixed seeds | every OS | random seeds, every OS |
| LazyFS power cut | | | Linux x64 | |
| Toxiproxy cut-offs and partitions | | fixed seeds | yes | random seeds |
| `kind` lead scenarios | | | Linux x64 | |

- **Set the model-based tests' case count per test**, with the `ProptestConfig` `goodfetch-test-support` builds for the running suite: 16 in Quick, 256 in the Pre-merge job that reruns them by a nextest filter, counted against the Pre-merge budget. Never set `PROPTEST_CASES`, which would lower every property test.
- **A failure in a deterministic nightly-only job is a bug, not flakiness.** A real-process or Toxiproxy failure goes through [Flaky tests and Live checks](#flaky-tests-and-live-checks) (diagnostic run, quarantine), and rule 3 of Regressions still applies when it does not reproduce. No release candidate is cut while either is open.
- **Over budget, shard, speed up or move a test to a later suite** (ADR 0061). Never lower case counts or seeds, and never drop a scenario, to fit.

### Regressions

1. A failing Simulation writes its shrunk Fault plan to a file and prints the command that replays it.
2. The fix commits the plan beside its scenario, with proptest's regression file. Committed plans run in the Pre-merge suite.
3. A failure from a non-deterministic layer (real processes, LazyFS, Toxiproxy, `kind`) is reproduced at the lowest deterministic layer that shows it, and that reproduction is the regression test. If none can, file an issue to extend the Simulation, and commit the failing seed to the real-process scenario meanwhile.
4. A TLC counterexample becomes a Fault plan and a fix to the model or design. A trace-validation failure is fixed in the code or in the stale model.
5. A test that once found a bug is never removed or weakened while its behaviour is still required; it may be replaced by a test covering the same failure.

## Flaky tests and Live checks

Decided in [ADR 0064](docs/adr/0064-tests-are-never-retried-and-unpinned-checks-run-as-live-checks.md). The Quick, Pre-merge and Release suites and the CI tiers are those of [ADR 0061](docs/adr/0061-three-test-suites-each-run-by-one-just-command-under-a-time-budget.md).

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

- **Each Live check has an owning area and a placement** listed in ADR 0064: real SaaS Sinks, installed Chrome's stable, beta and dev channels (pinned majors are tests), public testnets, the Clean-up rules' daily gate, the online conformance, the benchmark, the Cloud Run deploy, live-model evaluations, Scorecard, and fuzzing (ADR 0067).
- **Outcomes are pass, fail or inconclusive.** Inconclusive means the external side was unavailable; it never counts as a pass.
- **Results go to the results store** under the `live` tier, except fuzzing's, which stay private ([Security](#fuzzing)).
- **A fail, or three inconclusive runs in a row, opens or updates one `live-check` issue** in the current milestone, right after `flaky-test` issues on the frontier. If its cause is not stated within two business days, the next minor is not promoted until it is. Patches are not held, and the online conformance never blocks (ADR 0043). Fuzzing opens no `live-check` issue: its crashes stay private and block the next minor until fixed ([Security](#fuzzing)).
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
11. A new decoder, listener, Plugin kind, widening setting, Secret kind or output has its tests under [Security](#security).

## Suites, commands and time budgets

Decided in [ADR 0061](docs/adr/0061-three-test-suites-each-run-by-one-just-command-under-a-time-budget.md).

### The three suites

Each suite runs everything the suite before it runs.

| Suite | What it adds | Command | Runs in CI |
|---|---|---|---|
| Quick | Format and lint checks; in-process tests of what changed (Rust unit tests in `src/`, never `tests/it`); the committed format fixtures; `xtask codegen --check` and `xtask affected`'s own tests | `just quick` (`just quick all` for everything) | First job of every pull request, over the whole workspace |
| Pre-merge | The `tests/it` of every touched crate and its dependants; conformance on the local Proving ground; touched Sinks against their containers (every Sink until the merge queue exists, ADR 0032); touched SDKs, adapters and Integrations on one runtime; the Console on Chromium; docs examples; ADR 0032's static checks | `just pre-merge` (`just pre-merge all`) | Every pull request, on Linux |
| Release | Everything, for every package, on the full matrix of ADR 0024; deployment-target smoke runs; tests that need released binaries | `just release` (this host's cell), `just release-ci` (the whole matrix on CI) | Merge queue, nightly, every release |

- **A Quick-suite test starts no process, browser or container and binds no socket.** Anything that does is a `tests/it` test, or its runner's equivalent.
  _Why:_ the Quick suite is what you run after every edit, so it must stay seconds long.
- **Checks that are non-deterministic by nature are in no suite**: real SaaS Sinks, live models, the online Proving ground, real vendors, Chrome's beta and dev channels, the benchmark. They keep their own tiers.
- **The Release suite gates every merge**, through the merge queue. Before the merge queue exists, it runs on manual dispatch, and passes before each milestone's owner test and before any release.
  _Why:_ a macOS, Windows, arm64 or runtime failure found after the merge lands on `main`, which ships nightly.

### Commands

- **Run `just quick` before every push.** `just hooks` installs an opt-in pre-push hook that does it.
- **`just test <suite> [packages]`** runs one suite's tests without lints. CI jobs call it, and so can you.
- **`just fmt`, `just lint` and `just build`** format, check and build Rust, TypeScript and Python.
- **What changed comes from `xtask affected`**, in CI and locally. Until it exists, `just quick` and `just pre-merge` run over the whole workspace. Locally it diffs the working tree, uncommitted changes included, against the merge base with `origin/main`. Rust selects through `rdeps(…)`, so dependants run too.
- **A local run lists what it could not run on this host, and why**, such as containers without a container engine, or another OS. Its exit code covers what ran. CI is the verdict.
- **`just release-ci` dispatches the Release suite on your pushed branch** and follows it to the end. It never reaches the reference host.

### Time budgets

Budgets are the median (p50) of a job's last 20 runs in its own tier, on GitHub-hosted runners with warm caches: pull-request runs for Quick and Pre-merge; merge-queue runs (manual-dispatch runs while the repository is private), nightly and release runs for Release.

| Suite | Where | Budget | Hard cap |
|---|---|---|---|
| Quick | Locally, one crate changed, incremental build | ≤ 60 s (guidance) | none |
| Quick | CI, whole workspace | tests ≤ 2 min, job ≤ 10 min | 20 min |
| Pre-merge | Pull request | "all green" ≤ 20 min, each job ≤ 15 min | 30 min per job |
| Release | Merge queue | ≤ 60 min per group, each job ≤ 45 min | 90 min per job, 120 min for the queue |
| Release | Nightly and release pipelines | each job ≤ 3 h | 5 h per job |

- **Per-test caps stop hangs, never measure speed.** In the Quick suite a test is marked slow after 1 s and killed at 10 s; above it, slow after 30 s and killed at 5 min. nextest's `quick` and `ci` profiles, Vitest, Playwright and `pytest-timeout` all use these two levels.
- **No run fails for being slow within its cap.**
  _Why:_ benchmark suites on shared runners vary by about 2.7% run to run (CodSpeed's measurement), and whole jobs vary more, so a one-run timing gate would be flaky.
- **A breached budget opens an issue** through the budget check, run nightly and after every manual-dispatch Release run, labelled `needs-triage`. Fix it within two weeks: shard the job, speed up the tests, or move a test to a later suite with the reason in its pull request.
- **A stress run of new and changed tests, if the pull-request tier runs one, counts against the Pre-merge budget.** Shard it to fit; never lower its run count to meet a budget.

### Sharding, caching and reports

- **Shards are deterministic:** nextest archives once per target and splits with `--partition hash:m/n`; Vitest and Playwright use `--shard`; pytest uses `pytest-xdist` inside one job. Never split by recorded timings.
- **Caches save only from `main`**: a cache-warming job on every push to `main` builds without testing and saves `Swatinem/rust-cache`, the V8 build, pnpm's and uv's caches (keyed by their lockfiles); the nightly and manual-dispatch Release runs save the other targets'. The merge queue saves none, since pull requests can't restore its caches. Chromium, images and released binaries are cached by digest.
- **The warming job shares the pull-request jobs' rust-cache `shared-key` and profiles.** Linux pull-request caches take priority within GitHub's 10 GB; raising it is a paid resource the owner approves.
- **CI test builds use the `ci-test` Cargo profile** (`test` with `debug = "line-tables-only"`).
- **Every run writes JUnit XML**, local or CI. Every CI job uploads it, pass or fail, as `junit-<suite>-<tier>-<os>-<arch>[-<runtime>]-<shard>`, and a failing job writes a summary of its failed tests to its page.

## AI features and agent skills

Decided in [ADR 0065](docs/adr/0065-ai-features-are-tested-without-live-models-and-measured-by-oracle-graded-evaluations.md). Suites are the Quick, Pre-merge and Release suites of [ADR 0061](docs/adr/0061-three-test-suites-each-run-by-one-just-command-under-a-time-budget.md); Live checks are those of [ADR 0064](docs/adr/0064-tests-are-never-retried-and-unpinned-checks-run-as-live-checks.md), in no suite. A **live model** is one a provider serves and Goodfetch doesn't pin (a cloud model, directly or through a gateway); a pinned local model on a pinned server is a test input.

### Stand-ins for a model

| Layer | Stand-in | Suite |
|---|---|---|
| In-process AI logic | the Scripted model | Quick |
| Each adapter's wire format | Model recordings replayed by `httpmock` at the adapter's base URL | Pre-merge |
| The binary, journeys, AI end to end | the Proving ground's Fake model, in all three formats | Pre-merge |
| SDK code that calls a model | a model callback registered through the SDK handler | Pre-merge |
| Local model servers | the real server, pinned, with a pinned small model | Release (Linux cell) |

- **No test of an AI feature calls a live model.** Only the Live checks below reach one.
  _Why:_ providers no longer offer determinism: current Anthropic models refuse `temperature`, Gemini ignores it, and OpenAI's `seed` is best effort.
- **The Scripted model fails on a call beyond its script and on unused steps.** Script errors and faults as replies, and assert on the requests it recorded.
- **AI journeys name the Fake model as their Model provider.** Ask it for faults (rate limit, overload, timeout, cut stream, malformed JSON, refusal, wrong but plausible answer) through the task, never through a test-only switch in the binary.
- **Check Usage against the Fake model's own token count.**

### What an AI test asserts

- **Assert outcomes Goodfetch owns:** the Success check result, Grounding, the Plan file, the Effect journal, the Record, the Error code, Usage, what the oracle saw. Never a model's wording.
- **Never rely on temperature or a seed for determinism.**
- **Snapshot the whole request each AI operation sends, per format, with `insta`** (Quick). A prompt or tool-description change then shows as a reviewed diff.

### Model recordings

- **Record each cell of the matrix:**
  - per format on the direct provider: text, tool call and result turn, parallel tool calls, structured output, streaming of each, usage (cached and reasoning tokens), prompt caching, search grounding, image input, and the errors the engine branches on;
  - the named gateways, for the OpenAI-compatible format;
  - each hosted Decision provider (Jev, OpenAI's Decisions API): a choice, an abstention, a low-confidence choice, and its errors.

  A missing cell fails a Quick check.
- **Match on method, path and the canonical JSON body.** An unmatched request fails, and so does an unused recording.
  _Why:_ matching on the path alone replays a stale answer to a changed request.
- **Replay each stream whole and re-cut at seeded boundaries.** The stream parsers also have `proptest` and a fuzz target.
- **Keep only allowlisted headers at write time:** `content-type`, the version and beta headers the adapter sends, `retry-after` and the rate-limit headers the adapters read. Remove key query parameters, and replace request and organization IDs. A Quick check fails on anything else, or on anything shaped like a provider key.
- **Record only against Proving ground pages and tasks.**
- **Refresh on cause:** a changed request, a provider-check report, or 30 days before a manifest model's retirement. Use the recorder job, which opens a pull request, and review the diff. Never refresh on a timer or automatically.
- **Write faults that aren't worth recording as explicit `httpmock` stubs**, marked as synthetic.

### Pinned local models and the in-engine model

- **llama.cpp server and Ollama run in the Release suite's Linux cell**, each at a pinned version with a pinned small model downloaded by digest before ADR 0043's deny, with the dependency installs. vLLM joins them on its CPU build where it can serve the model.
  - Assert protocol behaviour only.
  - Send every sampling parameter the engine relies on.
  - Record the server version and model digest.
- **In the release pipeline**, every model on ADR 0012's tested list runs the same tests. vLLM runs there on a GPU host if its CPU build can't serve the model; that host is a paid resource the owner approves. LM Studio runs there where a runner supports its headless server.
- **The in-engine Decision provider model is checked on a labelled set with a margin:** on Linux in the Pre-merge suite, and on every OS in the Release suite. Never compare confidences exactly.

### Live checks that reach live models

- **The provider check runs nightly and compares shapes, never content.** It covers fields, types, stream events, error bodies and the headers adapters read, for every recorded cell: direct providers, gateways and hosted Decision providers. A difference opens or updates its `live-check` issue.
- **Every live job has its own credentials with a hard cap outside Goodfetch**: an Anthropic workspace spend limit, a gateway key limit, or a provider project budget that stops calls.
- **Every live job stops itself at its per-run cap.** The engine's runs use a currency Limit, Claude Code uses `maxBudgetUsd`, Codex uses a capped key, and the provider check and recorder compute their worst case before starting. Each job prints its worst case first.
- **Coding agents in live jobs make no other calls:** Claude Code runs with `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`, and Codex with its update check and analytics turned off.
  _Why:_ the job reaches only its named model hosts, and ADR 0043's deny refuses anything else.
- **Live jobs stay disabled until the owner approves the budget in ADR 0065.** Fork pull requests never receive model keys.

### Evaluations

- **Evaluations are `bench/` campaigns on the local Proving ground**, never tests. They come in three sizes: pull-request (only for changes to prompts, request snapshots, catalog descriptions or skills), nightly (plus a weekly run on larger models), and ADR 0025's release benchmark.
- **The oracle grades every trial.** No model grades an Evaluation. Record the engine's Success check and Grounding results beside the oracle's verdict.
- **Run 3 trials per task and model** at the pull-request and nightly sizes, each from a clean Goodfetch home. Every trial counts.
- **Report** the verified rate with its 95% Wilson interval, pass^3, false successes, safety violations, tokens and dollars per verified result, and the Success check and Grounding disagreements with the oracle. State the bound that "zero" can claim at the trial count (about 3/*n*).
- **A false success or a safety violation fails an Evaluation at any size** and blocks the next release candidate as a correctness failure.
- **The regression rule** is the mean over tasks of the paired difference in verified rate against the last accepted baseline on the same models. The difference is in percentage points (never a log ratio), with a block-bootstrap interval. It fails only when the interval's upper bound is below −Δ. An inconclusive result is reported, never a failure.
- **Δ is proposed at 10 points and waits for the owner's approval.** Until then, a regression opens or updates a `live-check` issue and blocks no release candidate.
- **A task joins its regression set at pass^3 ≥ 0.9 on every nightly model for five nights.** Add harder tasks once a regression set passes everything for 30 nights. Add a task for every AI bug fixed.
- **The pull-request size uses development seeds; nightly and release use held-out seeds.**
  _Why:_ prompts tuned on the tasks that judge them overfit.
- **Pin models in the manifest, and run old and new side by side for five nights before dropping one.** Record the model, any fingerprint the provider returns, the gateway, the local server's version and digest, the commit, the seed and the prompt snapshot hashes.

### Agent skills and tool descriptions

- **Skills are checked with no model:**
  - in Quick, `xtask codegen --check` and the Agent Skills frontmatter limits;
  - in Pre-merge, a check that every operation, command and option they name exists in the catalog, and a run of every example command against the local Proving ground, checked by the oracle.
- **If area 11 ships the skills as a Claude Code plugin, run `claude plugin validate --strict`** at a pinned version in Pre-merge.
- **Skill Evaluations run the CLI with and without the skills**, through promptfoo, on Claude Code (small, mid and large models) and Codex. The oracle grades; `skill-used` is a signal. Open an issue on any skill that doesn't beat its baseline on some model.
- **Snapshot the catalog's MCP projection per profile** (Quick). Keep each description at most 2,048 characters, and record each profile's total tool-definition size in the snapshot.
- **The tool-selection Evaluation checks a model's first tool call without executing it.**

### Manual tests

- **After a confirmed Evaluation regression, read the failed trials' Run traces** before accepting or fixing it.
  _Why:_ only a person can tell a wrong grade or a new failure mode from a real regression.
- **LM Studio**, where no CI runner can host its headless server: run the tested-list cases by hand before each release.

## Journeys

Decided in [ADR 0063](docs/adr/0063-end-to-end-tests-follow-a-catalog-of-numbered-journeys-kept-in-the-repository.md). A Journey is a goal a person reaches through Goodfetch's public surfaces; Journeys are the end-to-end tests.

### The catalog

- **One Journey per file, `journeys/J-NNN.toml`**, validated by `journeys/journey.schema.json`, which `xtask codegen` emits. Unknown fields fail.
- **A new Journey takes the number in `journeys/catalog.toml`** and increments it in the same pull request. If another pull request merged the same number first, take the next one.
- **Never change an ID on `main`, and never reuse one.**
- **Never change a Journey's `goal` under its ID.** Fix wording, steps, data or surfaces freely; for a different goal, set `status = "retired"` with `reason` and `replaced_by`, keep the file, and add a new Journey.
- **Write `goal` with RFC 2119 words**, readable as a specification without the code.
- **Write steps declaratively** ("export the Dataset as CSV"), never as clicks or flags, so the wording survives a change of implementation. Pin a step with `on` only when the goal needs that surface; give `manual` a reason only when no automated check can do it.
- **List every surface on which a person can reach the goal**, each with the milestone by which it must be proved, never earlier than the surface ships.
- **Fill `covers` with feature IDs** (`row:CO-18` for an ADR 0037 row).
- **Keep a Journey to its goal.** Edge cases, Error codes and limits go to unit, integration and conformance tests.
- **Each area's `/to-spec` adds the Journeys of its user stories**, and each ticket names the variants (`J-NNN@<surface>`) it makes pass.

### Data and isolation

- **Each Journey that uses Proving ground pages has its own `seed`.** Never share one between Journeys. A Journey that touches no page has none. `GOODFETCH_TEST_SEED` never overrides a Journey's seed.
- **Pin each Proving ground task a Journey names with its `digest`** from the task list. The digest ignores the port, so the same seed gives the same digest on every run.
- **Each variant run starts from scratch:** its own Proving ground on port 0 with the Journey's seed, its own Goodfetch home and cleared environment, `resolve` mapping the Proving ground's hosts to loopback, and its certificate authority trusted through the per-domain exception.
- **An AI Journey names the Fake model as its Model provider:** its variant's Proving ground serves it, and the setup steps point the Model provider setting at it through a public surface ([ADR 0065](docs/adr/0065-ai-features-are-tested-without-live-models-and-measured-by-oracle-graded-evaluations.md)). Never the Scripted model, never a live model.
- **Build the starting state through public surfaces** in the first steps. Never copy in a run file, database or home folder, except an older-format file for an upgrade Journey.
- **Run against the local copy only.** Online results stay conformance reports.
- **Drive the shipped artefacts**: the `goodfetch` binary and the SDK packages built from the commit, never an engine linked into the test.
- **When a generator change alters a task a Journey names, the digest check fails: check the Journey still fits, and update it, digest included, in the same pull request.** Task ordinals are append-only within a family; never reuse one.

### Tests and results

- **Carry the ID where the runner selects by it:**
  - nextest: a module `j002` (the ID lowercased, hyphen dropped) in the test's path;
  - Vitest: the tag `J-002`, declared from the catalog, `strictTags` on;
  - pytest: `@pytest.mark.journey("J-002")`, registered, `strict_markers` on;
  - Playwright Test: the tag `@J-002`.
- **Put a surface's Journey tests in a `journeys` folder of the package that ships it**, one file per Journey: `tests/it/journeys/j002.rs` in Rust, `tests/journeys/J-002.test.ts` (Vitest) or `J-002.spec.ts` (Playwright) in TypeScript, `tests/journeys/test_j002.py` in Python.
- **Start a variant's Proving ground, home and environment through test support** (`goodfetch-test-support`, or `goodfetch-test-env` through the `test-support/` packages), and drive the shipped binary.
- **Journey tests run with their package's integration tests**: in the Pre-merge suite when the package is touched, and in the Release suite.
- **One test per variant.**
- **Each variant run writes one result document with `kind = "journey"`.** Never set its outcome by hand: derive it from the steps, the checks and the oracle's answers, through the Rust constructor or a helper that passes the shared outcome vectors. A step not reached, or any Proving ground task not `verified`, fails the variant. Readers recompute the outcome and reject a document that disagrees, or whose steps, checks and named Proving ground tasks are not exactly its Journey's. The shared outcome vectors are in `test-support/fixtures/outcome-vectors/`.
- **The catalog check fails** on a file that does not parse, an ID that differs from its file name, repeats or is not below the next free number, a Proving ground task missing from the task list for the seed or whose digest differs, a test naming a retired or unknown Journey or an unlisted surface, two tests for one variant, a variant milestone earlier than its surface, or a retired Journey without a reason.

### Owner tests

- **List an owner test's "Journeys to try" from the catalog:** every variant of its milestone, by ID and title, with its manual steps.
- **An owner test is ready only when every automated variant of its milestone is `proved` on `main`.**
- **A finding on a goal no Journey covers adds a Journey.**

## Upgrades and version mismatches

Decided in [ADR 0066](docs/adr/0066-upgrades-are-tested-from-a-frozen-corpus-and-skew-from-every-released-minor.md). Suites are the Quick, Pre-merge and Release suites of [Test suites, local commands and time budgets](https://github.com/Jbmanllr/automan/issues/276).

### The format registry

- **Every versioned format is registered in `goodfetch-protocol`** with its version, schema and rule. A format read by an engine or spoken by a peer and missing from the registry is a bug.
  _Why:_ every check below is generated from the registry, so a registered format cannot miss one.
- **Expected outcomes come from the compatibility functions**, never from a hard-coded version pair. Each function is itself tested against a literal table.

### The Compatibility corpus

- **A new format lands with its `compat/<format>/head/` set and its scenario** in `xtask compat write`.
- **Never edit a fixture by hand.** Regenerate `head/` with `xtask compat write`; `xtask compat check` fails on drift.
- **Never change a frozen set** (`compat/<format>/<major>.<minor>/`). If the reading of an old file must change, change its snapshot in a reviewed pull request whose changelog entry says so.
  _Why:_ a frozen file is what a released version really wrote.
- **Change a format's schema only with a version bump** once a frozen set exists. `xtask compat check` fails otherwise.
- **Bump a format's version only on `main`.** A patch never changes one: `xtask release` compares a patch's `head/` versions and schemas with the `x.y.0` tag's.
- **Freeze from the release tag.** `xtask compat freeze --tag <tag>` copies the tag's `head/`, never `main`'s, which may hold unreleased versions. The release's entry in `compat/releases.toml` records its format versions, and `xtask compat check` requires a frozen set for each.
- **Fixtures cover the whole schema:** every property in a JSON document and a non-null value in every column. They hold synthetic data and Secret references only.
- **A corpus test is golden.** It reads, writes back and compares with its snapshot. A migrated SQLite store must also match a fresh store's schema. Checkpoints resume, Plans replay without a model, Recipes run and transcripts replay.
  _Why:_ "it opens" misses a misread.

### `goodfetch upgrade-files`

- **Each source in `compat/upgrade-paths.toml` is tested from every frozen set:** golden result, idempotent, the previous major's files unchanged, unrelated files untouched. A source not in the table and a newer major are refused.

### Refusals

- **Every refusal is tested from a fabricated version**, generated per format: a newer minor, an older major, a newer major, and each exception's own outcome (Usage-ledger family, Response cache, Clean-up rules, Operator policy). The Error code and message are snapshotted.
- **A handshake refusal is the reply to `hello`**, before any other message, and the connection then closes. Never test refusal with a time budget.
- **Test each handshake refusal twice:** in the Quick suite, in process, by sending the fabricated `hello` to the handshake code over an in-memory transport; in the Pre-merge suite, through the conformance harness's own `hello` to a running engine. No build carries a version override.
  _Why:_ the Quick suite starts no process and binds no socket.

### Released artifacts

- **Skew covers every released minor of the current major**, latest patch of each: SDKs, the Rust crate's client, the CLI and the MCP server against the current Daemon, the SDKs over stdio through `GOODFETCH_ENGINE_PATH` (every Attached engine applies the Daemon's rule at `hello`; this variable is how its version can differ from its SDK's), the Console, the Browser host and first-party Plugins.
- **An old Client runs its own release's Journeys** ([ADR 0063](docs/adr/0063-end-to-end-tests-follow-a-catalog-of-numbered-journeys-kept-in-the-repository.md)) from its tag, against the current binary given by path. Each variant keeps its own Proving ground and Goodfetch home. A Journey that asserts something outside semver is fixed on `main`, and its variant is excluded by variant ID (`J-NNN@<surface>`), with a reason, in `compat/skew-exclusions.toml`.
- **Rolling upgrades run under load** in a three-member group, once with the Lead Daemon last (a StatefulSet's default order with the Lead Daemon as pod 0) and once with it first, then a promotion from an older Group backup and the rollbacks.
- **The Helm upgrade starts from a release's chart,** installed by digest from `compat/releases.toml`, then runs `helm upgrade` to the current chart and `helm test`. Never `ct install --upgrade`.
  _Why:_ it upgrades from the git target branch and passes with a skip on a SemVer-breaking bump or an unchanged chart.
- **Older engines open the files the current engine wrote.** The Usage-ledger family opens; a current Checkpoint, Plan, settings file and Schedule file are refused with the upgrade message. Two majors run side by side on one machine.
- **Old artifacts come only from `compat/releases.toml`,** through the pinned artifact mirror, checked by digest and by `gh attestation verify --bundle --custom-trusted-root`, with the trusted root fetched before the outbound deny. One local command fetches the same set.
- **Never skip for a missing artifact.** A missing or mismatched artifact fails the job. An empty version list runs the current build against itself.
  _Why:_ a test that passes because nothing ran hides decay, as SQLite's and Cargo's skipped old-binary tests do.

### Suites

| Check | Suite |
|---|---|
| Corpus golden reads, fabricated refusals of files and of `hello` in process, compatibility-function tables, `upgrade-files` from fixtures, `xtask compat check` | Quick |
| Corpus replays, handshake refusals through the conformance harness, `upgrade-files` against the binary | Pre-merge |
| Every test that needs released artifacts: the skew matrix, rolling upgrades, older engines, two majors, the Helm upgrade | Release |

- **Tests that need released artifacts belong to the Release suite**, which the merge queue runs. Its nightly rerun and its run on each release's own bytes cover them; none is a nightly-only job.
- **Group each older minor's skew checks into as few jobs as fit the job budget.** One job per check exceeds the 20 concurrent jobs after two minors.
- **Never drop a minor, and never move a skew check out of the merge queue.** A Release suite budget breach is fixed with fewer or faster jobs per minor, or with more concurrency or larger runners, a paid resource the owner approves.
  _Why:_ the Release suite has no later suite to move to, and an untested minor decays.

## Security

Decided in [ADR 0067](docs/adr/0067-security-is-tested-per-trust-boundary-with-secret-canaries-and-private-fuzzing.md). The Quick, Pre-merge and Release suites are those of [Suites, commands and time budgets](#suites-commands-and-time-budgets): the Pre-merge suite runs on Linux, and the Release suite on the full matrix, in the merge queue once it exists and on manual dispatch before each milestone's owner test and before any release until then. Live checks are those of [Flaky tests and Live checks](#live-checks), in no suite.

### Trust boundaries

Every boundary has its tests. A decision that adds a boundary adds its row in the same change. The OSes column is the whole schedule: a test absent from an OS its row does not give is not skipped.

| Boundary | Tests | Suite | OSes |
|---|---|---|---|
| Untrusted decoders | corpus and crasher replay | Quick | every OS |
| | `cargo fuzz check` | Pre-merge | Linux x64 |
| | fuzzing | Live check, private repository | Linux x64 |
| Script sandbox | worker probes and inspection | Pre-merge; Release | Linux; every OS |
| | V8 memory-corruption scripts, on the test build fetched by digest | Release | Linux x64 |
| Plugin confinement | rule-to-probe check | Quick | every OS |
| | probes generated from each policy; engine refusals | Pre-merge; Release | Linux; every OS |
| Chromium's sandbox | renderer checks; startup probe cases | Pre-merge; Release | Linux; every OS |
| Egress | SSRF and hostile-input cases in-process | Quick | every OS |
| | the same cases end to end | Pre-merge; Release | Linux; every OS, rebinding end to end on Linux only |
| Endpoints and the peer link | endpoint table; protocol fuzzing; MCP conformance over HTTP | Pre-merge; Release | Linux; every OS |
| | ZAP baseline of the Console | Release | Linux |
| Secrets | redaction property tests | Quick | every OS |
| | Secret canary searches | the suite of the test that produces the output | its OSes |
| | `gitleaks` over job outputs | a step of every CI job | every OS |
| File outputs | path traversal cases | Pre-merge; Release | Linux; every OS, each OS's own cases |
| Audit log | tamper property tests | Quick | every OS |
| Reset gate | ADR 0001's planted Secret canaries | as ADR 0001 sets | as ADR 0001 sets |
| Unsafe code | ASan run of crates that opt into `unsafe` | Release | Linux glibc |
| Supply chain and workflows | ADR 0032's gates; image digest pins; CodeQL once public; `gitleaks` over commits until then | Pre-merge | Linux |

**Before the repository is public** (ADR 0032), the Release suite, with every part above, runs on manual dispatch before each milestone's owner test and before any release. CodeQL, GitHub secret scanning and push protection, and the nightly watch jobs start when the repository becomes public. Fuzzing starts when the owner approves its runner minutes.

### Fuzzing

- **Every decoder of input Goodfetch did not write itself has a `cargo-fuzz` target.** Today: JSON-RPC, plugin-protocol, peer-protocol and Script sandbox pipe framing; catalog-call arguments; CDP messages; HTML and Markdown; sitemaps, robots.txt and Site preference files; run-state import; Checkpoint and session-export bundles; Plugin manifests, OCI image indexes and bundle unpacking; Recipes and settings files; Filter lists; Session source formats, `Cookies.binarycookies` included; extraction schemas; URL canonicalisation and the resolved-IP classification.
- **Write each target's body as a plain function**, and replay its seeds and every past crasher from a `#[test]` on every OS.
  _Why:_ cargo-fuzz does not build on Windows, and path handling differs most there.
- **Check more than crashes where a pair exists:** round-trip for encoders and decoders, and the resolved-IP classification against the reference table built from the IANA registries.
- **Commit seeds and a dictionary per target.** The grown corpus stays private.
- **Never fuzz in a pull request.** Fuzzing is a Live check that runs only in a private repository, against the main repository's `main`: continuous builds and 600 s of ClusterFuzzLite `code-change` on the affected targets after every push, and `batch` for 3,600 s then `prune` and `coverage` nightly.
  _Why:_ a public job's logs and artifacts would disclose a crash already on `main` before its fix.
- **Keep fuzzing's results in the private repository.** They never go to the results store or its published report.
- **The job that builds and runs fuzz targets holds no secret** and a token with no permissions; a later job stores its results.
- **Run ClusterFuzzLite's images by digest** from Goodfetch's workflow, never through its action's `:v1` tag.
- **Send every new crash to a private security advisory draft** on the main repository, never a public issue. Until the main repository is public, which advisories need, open an issue in the private fuzz repository instead. A crash that also crashes the previous continuous build joins its advisory, or opens one. Fix it from the advisory's private fork (before publication, an ordinary pull request), and commit the crasher, minimised with `cargo fuzz tmin`, as a regression input with the fix.
- **An open fuzz crash blocks the promotion of the next minor** until it is fixed or triage closes it as not a vulnerability. Patches are not held. The private fuzz repository reports an open fuzz crashes status, red while any is open, and promotion to `x.y.0` waits for it, as for the Private component run.
- **Fuzz, and run the ASan tests, on one nightly toolchain pinned in its own file**, apart from `rust-toolchain.toml`.
- **Run the tests of every crate that opts into `unsafe` under AddressSanitizer** in the Release suite, on Linux glibc.

### Confinement

- **Generate Plugin confinement cases from the policy.** One probe per rule of each generated seccomp, Landlock, Seatbelt, AppContainer and broker policy, at each network level. A rule without a probe, or a probe without a rule, fails.
- **Assert the exact error** ADR 0033 gives for a call the seccomp, Landlock or Seatbelt policy refuses: a kill or an allowed call fails the test. Resource-limit probes expect the limit's own Error code.
- **Pair every refusal with a positive control:** the same probe first succeeds unconfined, in the same test.
  _Why:_ a probe that fails for an unrelated reason would otherwise pass as a refusal.
- **Assert the mechanism each runner uses** (Landlock ABI, user namespaces, mount-namespace fallback), so a silent fallback fails. A container under Docker's default seccomp profile on a kernel before 6.12 must refuse with `confinement_unavailable`.
- **Test the Script sandbox worker as if V8 were broken:** run the probes under the worker's own generated policy, and inspect a live worker (`Seccomp:` and `NoNewPrivs:` on Linux, an AppContainer token on Windows, the spec's method on macOS). In the Release suite, on Linux x64, run corruption scripts on a V8 build with `v8_enable_memory_corruption_api`; each must end as `script_crashed`.
- **Build the test V8 once per V8 version**, in its own required check on the pull request that changes the version, outside the Pre-merge suite's "all green" check. The job skips when that version's build is already published. A later job attests it and publishes it as a release asset, as the Goodfetch Chromium is. The author commits its digest, which a check matches against the attested asset. A fork's pull request cannot attest, so a V8 change from a fork is rebuilt from a branch of the main repository. The Release suite fetches the build by digest and never builds it.
- **Check that every renderer is sandboxed** for each browser Goodfetch launches: `Seccomp: 2` and `NoNewPrivs: 1` on Linux (Ubuntu 24.04 with its AppArmor restriction on), `chrome://sandbox` on Windows, Seatbelt on macOS. Test ADR 0049's startup probe in its refuse and record cases.

### Egress and hostile input

- **Keep SSRF cases in one table, as data, and run it at every layer:** the resolved-IP classification in-process, the HTTP tiers, the egress proxy, browser Sessions for every request type of ADR 0014, raw CDP, the relays of ADRs 0042 and 0044, a Borrowed browser, Plugin traffic, and Sinks and webhooks under "no private endpoints".
- **Cover every family:** encodings; IPv6 forms (IPv4-mapped, NAT64, 6to4, Teredo, zone IDs); every IANA special-purpose block that is not globally reachable; names with mixed answers; redirects of every kind and to non-HTTP schemes; rebinding; the upstream proxy path.
- **Assert that no connection reached the target**, from the Proving ground's connection-recording fixture, and the typed refusal.
- **Test rebinding against the Proving ground's DNS fixture**: end to end on Linux in a network and mount namespace whose resolver points at it; in-process on every OS with a scripted sequence of answers. Never use a public rebinding service.
- **Keep the IANA special-purpose registries committed with their date.** A nightly watch job, which runs none of the product, opens an issue when they change.
- **Test the hostile-input set where each case can happen:** CR, LF or NUL in header values (Recipes, Sinks, webhooks, CDP); a domain-bound Secret across a cross-origin redirect; path traversal into every file Goodfetch writes from outside input, with each OS's cases; external `$ref` in extraction schemas; no process spawned to compare page content (checked under a seccomp filter that refuses `execve`); decompression bombs, endless bodies, deep nesting and oversized messages ending in their typed limit errors.

### Endpoints

- **Run the endpoint table against every listener:** the Console, the Daemon's socket and WebSocket, the HTTP listener, the ops listener, MCP over HTTP, the Live view, raw CDP, the Attached engine's private endpoint, an Exposed Daemon and the peer link.
- **Cover every row:** missing, wrong, expired, revoked, out-of-scope and other-listener tokens, and tokens in a query string; a foreign `Host`; a foreign or missing `Origin` on the request and the WebSocket handshake; plain HTTP where TLS is configured; `X-Forwarded-*` from an unconfigured address; each listener's own rules (ADRs 0031, 0049, 0050, 0055, and MCP's audience and no passthrough). Check each refusal in the Audit log.
- **Fuzz the protocol from the committed JSON Schema bundle** with `proptest` and `proptest-state-machine`, over every transport. Every invalid call gets a typed error and changes no state.
- **Run the MCP conformance suite against MCP over HTTP.**
- **Run a ZAP baseline scan of the Console in the Release suite:** image by digest, `-silent`, `allow_issue_writing: false`, FAIL rules committed, the crawl bounded by depth rather than time, under ADR 0043's deny so its Egress canary catches a call home.

### Secrets

- **Plant a Secret canary for every kind of secret** (typed password, TOTP seed, proxy password, model API key, webhook signing secret, Sink credential, Session cookie, endpoint token, run data key), through the Proving ground's login flows and every Secret provider.
- **Search every output for every Secret canary**, raw and JSON-escaped, URL-encoded, base64 and hex. The outputs are:
  - logs, Run traces, HAR files and console captures, and screenshots and recordings where masking applies;
  - the Audit log, the Effect journal, and Checkpoints outside their encrypted fields;
  - Exports and Notifications;
  - JSON-RPC errors, error messages in run state, and model requests;
  - the Console's offline cache, `goodfetch doctor`'s file, and the SDKs' `repr`, `toString` and errors.

  _Why:_ redaction is best effort against arbitrary transforms, but Goodfetch's own encodings are known.
- **Property-test redaction** with generated values that are hard to redact (quotes, backslashes, newlines, `%`, Unicode, very long, empty, common words, values that contain one another), through every formatter and serializer. Commit `proptest-regressions/`.
- **Scan each CI job's logs, test reports, snapshots and artifacts with the `gitleaks` binary** (pinned by checksum, Secret canaries added as a rule) before they upload. A hit fails the job.
- **Scan commits,** Model recordings included: with GitHub secret scanning and push protection once the repository is public; until then with `gitleaks` over each pull request's commits in the Pre-merge suite.

### Other rules

- **Property-test the Audit log's hash chain:** an edit, deletion, reordering or truncation of entries or of the tail fails verification, a Daemon group's single chain included.
- **Add one regression test per bypass** at the layer it slipped through, Goodfetch's own or a class a comparable project disclosed. Cite the advisory in a comment. A nightly watch job lists new advisories of comparable projects and opens an issue.
- **Never skip a security test where it is scheduled.** The OSes in the boundary table are the schedule, and a test's absence from an OS its row does not give is not a skip. On a scheduled host, never `#[ignore]` it, return early or gate it at run time; where the host lacks a mechanism, assert the refusal (`confinement_unavailable`, `local_address_unavailable`). A missing tool fails the job.
- **Pin every container image a workflow runs by digest.**
- **Run CodeQL** on Rust, TypeScript, Python and the workflows in the Pre-merge suite once the repository is public. A high or critical alert blocks the merge.

### Manual security tests

Only these stay manual. They are not Journeys: none has an automated step, and each runs on a release candidate rather than at a milestone's owner test. They are named by title.

| Manual test | When | What a person does | Recorded on |
|---|---|---|---|
| Threat-model review | each minor's release candidate | Walks the trust-boundary table against the minor's changes and confirms each new decoder, listener, Plugin kind, widening setting, Secret kind and output has its test | the release pull request |
| Vulnerability triage | each new fuzz crash, external report and advisory-watch issue | Judges it against `SECURITY.md`'s vulnerability definition; opens the private advisory or closes with a reason | the advisory or issue |
| Independent security assessment | before 1.0 and each later major; paid, owner-approved | An outside tester attacks the boundaries; each finding gets a fix and a regression test | a report linked from the release notes |
| Confinement where CI cannot run | each minor's release candidate | WSL1's refusal; Bridge Sinks under Docker Desktop and Podman machine on macOS and Windows; the documented host-firewall recipes for `direct` | the release pull request |
| Passkeys on real authenticators | each minor's release candidate | Sign-in, step-up and recovery on a platform authenticator and a hardware key | the release pull request |
