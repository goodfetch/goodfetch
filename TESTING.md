# Testing

How Goodfetch is tested: every surface, at every level. Each rule comes with its reason, and the hard-to-reverse ones are decided in ADRs. [`STYLE.md`](STYLE.md) shapes the code these tests drive. Here a *snapshot* is a stored expected value, never a Checkpoint, a Group backup or a Reference screenshot.

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
17. [SDK and CLI tests](#sdk-and-cli-tests)
18. [MCP, framework adapters and Integrations](#mcp-framework-adapters-and-integrations)
19. [Performance budgets](#performance-budgets)
20. [Security](#security)
21. [Console and docs site](#console-and-docs-site)
22. [Feature coverage](#feature-coverage)
23. [Accessibility, localization and visual regression](#accessibility-localization-and-visual-regression)
24. [Which stories need a Journey](#which-stories-need-a-journey)
25. [Goodfetch Chromium](#goodfetch-chromium)
26. [Free tiers and local secrets](#free-tiers-and-local-secrets)

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
- **Tests of a Product's executable live in its crate's `tests/it`** and find the executable through `CARGO_BIN_EXE_goodfetch-collie` or `CARGO_BIN_EXE_goodfetch-foxhound`, never a guessed path.
  _Why:_ the build directory's layout may change without notice.
- **Foxhound's tests live in Foxhound's crates.** No crate outside `crates/foxhound/` but `xtask` takes a Foxhound crate, even as a dev-dependency, and the workspace-wide `cargo-deny` check refuses one (ADR 0087).
  _Why:_ Collie's tests then build and pass without Foxhound's code.
- **Each Product's executable crate tests its composition** in the Quick suite, in process: exactly one handler for each operation of its Product's catalog, and Foxhound's composition contains all of Collie's (ADR 0087).
  _Why:_ the composition is a hand-kept list, and Foxhound must carry everything Collie carries.
- **Python tests run with `--import-mode=importlib` and `strict = true`** in `[tool.pytest]`.
  _Why:_ tests outside the package run against the installed wheel, as pytest recommends.

### Which Product's executable a test runs

Decided in [ADR 0094](docs/adr/0094-both-products-are-released-and-signed-together-and-each-test-runs-on-collie-unless-it-needs-foxhound.md).

- **A test that runs an executable runs Collie's when Collie carries everything it needs, and Foxhound's otherwise:** once, never once per Product. A test that needs Foxhound anywhere in its setup or steps, such as a rolling upgrade under a Partitioned crawl, runs on Foxhound's.
  _Why:_ Foxhound's executable links Collie's crates, so a test of Collie's covers the code users of either Product run, and the containment checks below cover what Foxhound adds around it.
- **A test that measures an executable itself runs on each Product's:** its size (ADR 0087), its startup benchmarks and its idle memory.
- **Each package is tested against its own Product's executable:** each executable crate's `tests/it` (its CLI, MCP server, exit codes and `--help` tree), and each Product's SDK packages, Console build, image and Helm chart.
- **One test build per target serves both Products:** the in-process tests and both executables come from the same workspace invocation. Never add a test build per Product.
  _Why:_ Cargo unifies features across the packages of one invocation, so Collie's executable in a test links the same shared code as Foxhound's, and a second build would compile again every crate whose features differ. Each Product's own `cargo build -p` (ADR 0087) is run from its own bytes by the release before it publishes.
- **Foxhound proves that it contains Collie at the seams where its executable is composed,** never by running Collie's tests again:
  1. its engine composition contains Collie's (ADR 0087's composition test);
  2. a Quick check fails unless every generated contract of Collie's (catalog entries, setting keys, Error codes, events, CLI commands with their flags, MCP tools) is in Foxhound's, the same;
  3. a served test on Foxhound's executable visits every route of Collie's Console build and checks that it renders;
  4. each Collie Journey's `cli` variant also runs on Foxhound's executable, as `J-NNN@foxhound-cli` ([Journeys](#journeys)).
- **A conformance suite runs on the smallest Product that carries its contract:** browser conformance and the Plugin kinds Collie carries on Collie's executable, Sinks and Fetch backends on Foxhound's. Of the [Session ownership and proxy limit cases](#sdks), those on a Session a program or an Agent opened and on a Collie Run's proxy limits run on Collie's, and the one on a crawl's Session pool on Foxhound's.

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
- **turmoil's network adapter is the one shared Rust helper outside `goodfetch-test-support`:** a test-only crate of its own that implements `goodfetch-engine`'s network interface, taken only by integration tests (`tests/it`), never by a unit test (ADR 0087).
  _Why:_ an integration test links the engine once, where a unit test of `goodfetch-engine` would see two copies of its types; one rule for every crate is simpler.
- **The test versions of `Clock` and `Entropy` are `Clock::paused(start)` and `Entropy::seeded(seed)`**, in the crate that defines the handles. `disallowed-methods` refuses both; each test module or integration-test root that calls them opts in with `#[expect(clippy::disallowed_methods, reason = "…")]` (`#![expect]` in `tests/it/main.rs`), never with a crate-wide or `cfg_attr(test, …)` attribute in product code. `main` builds only the system versions. Tests that pause Tokio's clock enable its `test-util` feature under `[dev-dependencies]`. The seed and the start time come from `goodfetch-test-support`.
- **TypeScript and Python reuse the Rust setup, never rewrite it.** The `goodfetch-test-env` executable of `goodfetch-test-support` prepares a test's folder, home and environment, starts what it is asked to, prints one JSON description (paths, environment map, seed, addresses), and tears everything down when its standard input closes.
  - `@goodfetch/test-support` (`test-support/typescript/`, private) gives Vitest fixtures through `test.extend`.
  - `goodfetch-test-support` (`test-support/python/`, never published) is a pytest plugin registered through `pytest11`.
- **The `test-util` feature of `goodfetch-collie` and `goodfetch-foxhound` is for people who embed them**, never Goodfetch's own helper home.
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
- **Doctests only on `goodfetch-collie` and `goodfetch-foxhound`**, where they are the examples of ADR 0030.

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

- **A dependency with a local stand-in gets no handle.** Only `Clock` and `Entropy` are swapped in tests, and, in the [Simulation](#simulation) alone, `transport`'s network (ADR 0060). That network is a value with product adapters, the OS and the in-memory pair the Products' embeddable crates use, so its interface is public for them, not for tests, and it is not a third handle.
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
2. A per-test temporary `GOODFETCH_HOME`, in a folder whose name holds a space and a non-ASCII letter on every OS ([Accessibility, localization and visual regression](#accessibility-localization-and-visual-regression)).
3. A child process starts with a cleared environment whose `HOME`, `USERPROFILE` and `XDG_` variables point inside the test's folder, and whose `TZ` and `LANG` are `UTC` and `C.UTF-8` unless the test names others.
4. No fixed ports: bind port 0 and report the address, or use a Unix socket or named pipe in the test's folder.
5. One Valkey per CI job, with a namespace per test.
6. Resources shared by the whole user (keychain, service manager) run in a nextest test group, with unique names.
7. Certificate authorities are made per test: the Proving ground's from its seed, a Daemon group's by `group init` into the test's folder.
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
- **Never build a runtime in the engine.** Only `main`, test helpers and bench targets (ADR 0071) build one, plus the Rust SDK's connection through the helper `main` uses; its unit tests over the in-memory transport pair pass it the test helper's paused current-thread runtime, and its linked-mode tests use the production runtime with short durations (ADR 0081). `tokio::task::block_in_place` is in `disallowed-methods`.
- **Give every third-party time hook the `Clock`** (rustls's `TimeProvider`), and the test `tracing` subscriber's timer too, so traces carry simulated time.
- **Emit each transition a TLA+ model names as a `tracing` event** under the model's target, with the model's variable names as fields.
- **No failpoints.** Inject faults at I/O seams: the crash VFS, the transport adapters, the plugin protocol's in-memory adapter, Toxiproxy, process kills.
  _Why:_ a crash only matters at an I/O boundary, and each boundary already has a seam.

### Simulation

- **Several hosts in one process run on turmoil**, pinned, each on its own paused current-thread Tokio runtime with `Clock::paused` and `Entropy::seeded`, built with `--cfg tokio_unstable` so each host's runtime randomness (`select!` order) follows the seed. No crate swaps the runtime.
- **Compile the scenarios only under `cfg(tokio_unstable)`** (`#[cfg(tokio_unstable)] mod simulation;` in the `tests/it/main.rs` of each crate that has them). They run from a nextest archive and CI job of their own, filtered to the `simulation` module, with their own rust-cache key, counted against the Pre-merge budget and ADR 0061's cache priority.
  _Why:_ every other test then runs on the Tokio configuration production ships.
- **Derive every seed from the test's seed**, which `goodfetch-test-support` supplies (`GOODFETCH_TEST_SEED` overrides it): one stream for turmoil, one per host's `Entropy`, one for the Fault plan's generation.
- **A scenario keeps an Observation log** of operations, answers and faults, and **the shared invariant checkers judge it**: fencing, one lead, Limits, Partitions, the Audit log chain (through the product's own verifier, passed in as a value), and liveness within a stated simulated time once faults stop. Real-process tests and conformance suites use the same checkers.
- **Faults come from Fault plans** generated by proptest-state-machine and shrunk by proptest on failure.
- **Check determinism:** rerun a few seeds and compare Observation logs and `tracing` output. Fix any mismatch at its source.
- **Leave security values out of the comparison.** Keys, tokens, certificates and TLS handshakes still come from the OS generator (ADR 0059) and are recorded redacted. If behaviour depends on their bytes, fix the code; never seed them.
- **The Politeness store is not simulated.** Group scenarios set `politeness.sharing: per_daemon`.

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

- **Simulation scenarios:** the `tests/it/simulation/` of the crate that holds the behaviour, one module per behaviour area: `goodfetch-engine`'s, or a Foxhound crate's for Partition handoff, the crawl's work-item leases and Sink delivery (ADR 0087).
- **turmoil's network adapter:** a test-only crate outside `crates/foxhound/`, which `goodfetch-engine`'s and Foxhound's tests take as a dev-dependency (ADR 0087).
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
- **The no-retry check in the Quick suite refuses all of the above**, and the flags and variables that undo them (nextest `--retries`, `--flaky-result`, `NEXTEST_RETRIES`, `NEXTEST_FLAKY_RESULT`; Vitest `--retry`; Playwright `--retries`; Chromium's `--test-launcher-retry-limit` other than 0, a gtest run without it, and `--num-retries`; wptrunner's `--retry-unexpected` above 0, `--rerun` above 1 and `--repeat-until-unexpected`; typ's `--retry-limit` and `--isolated-script-test-launcher-retry-limit` other than 0, `--retry-only-retry-on-failure-tests`, and a `RetryOnFailure` expectation). Downloads before the deny of ADR 0043 may retry: they test nothing.
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
  - these and the diagnostic runs share a ceiling of 1,000 included minutes a month. At the ceiling, constrained and then diagnostic runs are skipped until the month ends. The coverage and in-diff mutation jobs of [Feature coverage](#feature-coverage) count toward it too, and stop once the month's count reaches 550 minutes.
  - Once the nightly tier is on, both run every night with no ceiling.

### A flaky test found after merge

1. CI opens (or reopens) one `flaky-test` issue for it, in the current milestone.
2. The same day, add the test to the quarantine list with its issue.
3. If the test or the code it covers changed in the last 7 days, you may also revert that change: revert the product code, and the tests stay quarantined.
4. `flaky-test` issues come first on the frontier.
5. Fix the cause. A longer timeout alone, a sleep or a retry is not a fix. A race or leak in the product is fixed in the product, and the test stays as it was.
6. The fix takes the test off the list after 200 passing runs; the issue closes when it merges.

- **The quarantine list is one file in the repository** (upstream Chromium tests the Chromium build gate excludes are not on it: see [Goodfetch Chromium's expected results](#expected-results)). Each entry has a test ID, its open `flaky-test` issue and a date; the Quick suite's check refuses an entry without an open issue or naming a test that no longer exists.
- **Quarantined tests still run** 50 times a day in the daily test-health job (the 10 oldest entries), into the results store.
- **A quarantined test blocks** (unlike an upstream exclusion of [Goodfetch Chromium's expected results](#expected-results)) its milestone's gate, every release (`xtask release` refuses while the list is not empty), and counts its feature as untested.
- **A flaky test is never removed or weakened to make a suite pass:** quarantine it, fix it, or replace it with a test covering the same behaviour (for example by moving its unpinned part to a Live check).

### Live checks

- **Each Live check has an owning area and a placement** listed in ADR 0064: real SaaS Sinks, installed Chrome's stable, beta and dev channels (pinned majors are tests), public testnets, the Clean-up rules' daily gate, the online conformance, the benchmark, the Performance series (ADR 0071), the Cloud Run deploy, live-model evaluations, Scorecard; and, from ADR 0070, the newest MCP conformance suite, Inspector and SDK clients, each agent framework's newest releases, the n8n scanner on the published node, and real Integration platform accounts; and fuzzing (ADR 0067); and, from ADR 0068, the docs site's `docs-external-links` (nightly, or before each release candidate while the nightly tier is off) and `docs-deploy-smoke` (after each production deploy), which block nothing directly ([Console and docs site](#console-and-docs-site)); and, from ADR 0073, `screen-reader` (nightly, or before each release candidate while the nightly tier is off), blocking nothing directly; and, from ADR 0074, the Chromium patch preview and the Chromium build currency, owned by area 8, which block nothing directly ([Goodfetch Chromium](#goodfetch-chromium)); and, from ADR 0089, `lend-libraries-newest` and `crawler-examples-newest`, owned by area 11 (nightly, or before each release candidate while the nightly tier is off), which block nothing directly ([SDKs](#sdks)).
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
12. Each declared Feature is one the test asserts ([Feature coverage](#feature-coverage)).
13. Each mutant that survives in changed code is killed, or the Verification section explains it.
14. Each new exception names evidence no pinned test can replace.
15. A spec change and its `features/stories/` file agree.
16. A new Console string is in the catalog, a new component state has its Storybook story, and every changed Reference screenshot is intended ([Accessibility, localization and visual regression](#accessibility-localization-and-visual-regression)).
17. A new or changed Chromium patch has its row in `chromium/surfaces.toml` and its tests, and each new expectation entry has its category and evidence ([Goodfetch Chromium](#goodfetch-chromium)).

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
- **The Chromium build gate holds pinned tests in no suite** ([Goodfetch Chromium](#goodfetch-chromium)), because they need the build's source tree. Like the test V8 build job ([Security](#confinement)), it is its own required check beside "all green".
- **Checks that are non-deterministic by nature are in no suite**: real SaaS Sinks, live models, the online Proving ground, real vendors, Chrome's beta and dev channels, the benchmark, the Performance series. They keep their own tiers.
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
- **The warming job shares the pull-request jobs' rust-cache `shared-key` and profiles.** Linux pull-request caches take priority within GitHub's 10 GB; a larger cache is declined while there is no revenue (ADR 0076), so fit within it.
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

  A missing cell fails a Quick check. While there is no revenue ([ADR 0076](docs/adr/0076-tests-and-ci-run-on-free-tiers-and-ai-calls-go-through-the-vercel-ai-gateway.md)), cells are recorded through the Vercel AI Gateway and labelled `via gateway`, which counts; the direct-provider, other-gateway and hosted Decision provider cells, and Gemini's native format, which the gateway does not serve, wait and are reported missing without failing.
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
- **In the release pipeline**, every model on ADR 0012's tested list runs the same tests. vLLM runs there on its CPU build; a GPU host is declined while there is no revenue (ADR 0076), so where the CPU build can't serve the model, vLLM waits. LM Studio runs there where a runner supports its headless server.
- **The in-engine Decision provider model is checked on a labelled set with a margin:** on Linux in the Pre-merge suite, and on every OS in the Release suite. Never compare confidences exactly.

### Live checks that reach live models

- **The provider check runs nightly and compares shapes, never content.** It covers fields, types, stream events, error bodies and the headers adapters read, for every recorded cell: direct providers, gateways and hosted Decision providers (while there is no revenue, only the cells recorded `via gateway`; ADR 0076). A difference opens or updates its `live-check` issue.
- **Every live model call goes through the Vercel AI Gateway on its free tier** ([ADR 0076](docs/adr/0076-tests-and-ci-run-on-free-tiers-and-ai-calls-go-through-the-vercel-ai-gateway.md)): one key, `AI_GATEWAY_API_KEY`, free-tier models only, no credits bought and auto top-up off, so no job can spend money. A `429` or an exhausted free credit makes the run inconclusive, never a purchase.
- **Every live job stops itself at its per-run cap.** The engine's runs use a currency Limit, Claude Code uses `maxBudgetUsd`, Codex uses a capped key, and the provider check and recorder compute their worst case before starting. Each job prints its worst case first.
- **Coding agents in live jobs make no other calls:** Claude Code runs with `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`, and Codex with its update check and analytics turned off.
  _Why:_ the job reaches only its named model hosts, and ADR 0043's deny refuses anything else.
- **Live jobs run on the gateway's free tier from M4.** Fork pull requests never receive model keys.

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
- **Pin models in the manifest, chosen from the gateway's free tier, and run old and new side by side for five nights before dropping one.** An Evaluation or skill Evaluation whose model is not in the free tier waits, reported inconclusive, until revenue (ADR 0076). Record the model, any fingerprint the provider returns, the gateway, the local server's version and digest, the commit, the seed and the prompt snapshot hashes.

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
- **Name the Journey's `product`:** `collie` when Collie carries every step, `foxhound` otherwise. Its surfaces are that Product's.
- **A Collie Journey with a `cli` surface also runs as `J-NNN@foxhound-cli`:** the same test, given Foxhound's executable, in the same suites. It is the only variant that names a Product.
  _Why:_ it shows end to end that Foxhound's executable keeps Collie's behaviour, where the other containment checks show only that Collie's definitions are there.
- **Fill `covers` with Feature IDs** (`row:CO-18` for an ADR 0037 row; every kind is in [Feature coverage](#feature-coverage)).
- **Keep a Journey to its goal.** Edge cases, Error codes and limits go to unit, integration and conformance tests.
- **Each area's `/to-spec` adds the Journeys of its stories with `journey = true`** ([Which stories need a Journey](#which-stories-need-a-journey)), and each ticket names the variants (`J-NNN@<surface>`) it makes pass.

### Data and isolation

- **Each Journey that uses Proving ground pages has its own `seed`.** Never share one between Journeys. A Journey that touches no page has none. `GOODFETCH_TEST_SEED` never overrides a Journey's seed.
- **Pin each Proving ground task a Journey names with its `digest`** from the task list. The digest ignores the port, so the same seed gives the same digest on every run.
- **Each variant run starts from scratch:** its own Proving ground on port 0 with the Journey's seed, its own Goodfetch home and cleared environment, `resolve` mapping the Proving ground's hosts to loopback, and its certificate authority trusted through the per-domain exception.
- **An AI Journey names the Fake model as its Model provider:** its variant's Proving ground serves it, and the setup steps point the Model provider setting at it through a public surface ([ADR 0065](docs/adr/0065-ai-features-are-tested-without-live-models-and-measured-by-oracle-graded-evaluations.md)). Never the Scripted model, never a live model.
- **Build the starting state through public surfaces** in the first steps. Never copy in a run file, database or home folder, except an older-format file for an upgrade Journey.
- **Run against the local copy only.** Online results stay conformance reports.
- **Drive the shipped artefacts**: the Products' executables and the SDK packages built from the commit, never an engine linked into the test.
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
- **The catalog check fails** on a file that does not parse, an ID that differs from its file name, repeats or is not below the next free number, a Proving ground task missing from the task list for the seed or whose digest differs, a test naming a retired or unknown Journey or an unlisted surface (`foxhound-cli` counts as listed only on a Collie Journey that lists `cli`, with `cli`'s first milestone), two tests for one variant, a variant milestone earlier than its surface, or a retired Journey without a reason.

### Owner tests

- **List an owner test's "Journeys to try" from the catalog:** every variant of its milestone, by ID and title, with its manual steps, plus the milestone's `manual` exceptions of [Feature coverage](#feature-coverage).
- **An owner test is ready only when every automated variant of its milestone is `proved` on `main`,** and no Feature due at its milestone is untested ([Feature coverage](#feature-coverage)).
- **A finding on a goal no Journey covers adds a Journey.**
- **An owner test installs both Products' executables** from a local build, and its list holds the Journeys of both Products.

## Upgrades and version mismatches

Decided in [ADR 0066](docs/adr/0066-upgrades-are-tested-from-a-frozen-corpus-and-skew-from-every-released-minor.md). Suites are the Quick, Pre-merge and Release suites of [Test suites, local commands and time budgets](https://github.com/goodfetch/goodfetch/issues/276).

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

### `upgrade-files`

- **Each source in `compat/upgrade-paths.toml` is tested from every frozen set:** golden result, idempotent, the previous major's files unchanged, unrelated files untouched. A source not in the table and a newer major are refused.

### Refusals

- **Every refusal is tested from a fabricated version**, generated per format: a newer minor, an older major, a newer major, and each exception's own outcome (Usage-ledger family, Response cache, Clean-up rules, Operator policy). The Error code and message are snapshotted. Between an engine and a Browser host only another major is refused: a fabricated `hello` of another minor checks that both sides settle on the lower one, and a Session needing more than an older Browser host's minor checks `browser_host_outdated` (ADR 0096).
- **A handshake refusal is the reply to `hello`**, before any other message, and the connection then closes. Never test refusal with a time budget.
- **Test each handshake refusal twice:** in the Quick suite, in process, by sending the fabricated `hello` to the handshake code over an in-memory transport; in the Pre-merge suite, through the conformance harness's own `hello` to a running engine. No build carries a version override.
  _Why:_ the Quick suite starts no process and binds no socket.

### Released artifacts

- **Skew covers every released minor of the current major**, latest patch of each: SDKs, each Product's Rust crate's client, the CLI and the MCP server against the current Daemon, the SDKs over stdio through their Product's engine-path variable, `GOODFETCH_COLLIE_ENGINE_PATH` or `GOODFETCH_FOXHOUND_ENGINE_PATH` (every Attached engine applies the Daemon's rule at `hello`; this variable is how its version can differ from its SDK's), the Console, the Browser host in both directions (older engines against the current Browser host, and the current engine against older Browser hosts, which settle on the lower minor) and first-party Plugins.
- **An old Client runs its own release's Journeys** ([ADR 0063](docs/adr/0063-end-to-end-tests-follow-a-catalog-of-numbered-journeys-kept-in-the-repository.md)) from its tag, against the current binary given by path. Each variant keeps its own Proving ground and Goodfetch home. A Journey that asserts something outside semver is fixed on `main`, and its variant is excluded by variant ID (`J-NNN@<surface>`), with a reason, in `compat/skew-exclusions.toml`.
- **Skew runs per Product:** each Product's Clients, SDKs over stdio and Console build against the current executable of the same Product, and each old Journey against the current executable of its Product. An older Collie opens the current files of the formats both Products carry; an older Foxhound, those of Foxhound's.
- **Test a pair across Products where a promise covers it:** the Product check of `hello` that [Protocol, Operation catalog and SDKs across two products](https://github.com/goodfetch/goodfetch/issues/458) adds, for every pair of Products, with fabricated `hello`s as version refusals are; and, where [Collie and Foxhound installed on one machine](https://github.com/goodfetch/goodfetch/issues/486) lets two installs share a file, an older Collie opening the one the current Foxhound wrote. A released Collie Client is not run again against Foxhound's executable: the containment checks show Foxhound serves Collie's operations unchanged.
- **Rolling upgrades run under load** in a three-member group, once with the Lead Daemon last (a StatefulSet's default order with the Lead Daemon as pod 0) and once with it first, then a promotion from an older Group backup and the rollbacks.
- **The Helm upgrade starts from a release's chart,** installed by digest from `compat/releases.toml`, then runs `helm upgrade` to the current chart and `helm test`. Never `ct install --upgrade`.
  _Why:_ it upgrades from the git target branch and passes with a skip on a SemVer-breaking bump or an unchanged chart.
- **Rolling upgrades and the Helm upgrade run on Foxhound,** whose load includes a Partitioned crawl. A Quick check renders Collie's and Foxhound's charts with `helm template` and fails on any difference but the image.
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
- **Never drop a minor, and never move a skew check out of the merge queue.** A Release suite budget breach is fixed with fewer or faster jobs per minor, or with more concurrency on standard runners; larger runners are declined while there is no revenue (ADR 0076).
  _Why:_ the Release suite has no later suite to move to, and an untested minor decays.

## SDK and CLI tests

Decided in [ADR 0069](docs/adr/0069-sdks-and-the-cli-are-tested-against-a-scripted-engine-and-the-binary-of-the-same-commit.md).

### SDKs

- **Unit tests run against the Scripted engine in-process**, through an in-memory transport; integration tests and Journeys run the Product's executable built from the same commit. `sdk-rust` Journeys run with a Daemon only; `linked` is never a Journey mode.
  _Why:_ the binary can't fabricate a refused `hello` or a crash mid-call, and a fake alone passes where the engine fails.
- **Scripts live only in `test-support/fixtures/engine-scripts/`**, never inline in a test. Every message step in them must validate against the committed JSON Schema bundle; only raw-byte steps, deliberately not JSON-RPC, are skipped.
  _Why:_ one validated corpus keeps the stand-in true to the engine for every SDK at once.
- **Pass the Product's engine-path variable and `GOODFETCH_HOME` in the SDK's environment-map option**, as the test-support fixtures build it from `goodfetch-test-env`. Never set them in the test's own process; an Attached engine gets exactly that map as its environment.
- **Process faults use `goodfetch-scripted-engine`**, reached as an Attached engine through the Product's engine-path variable (TypeScript and Python) or as a Daemon through a runtime file in the test's Goodfetch home (every SDK). No build of either Product carries a fault switch.
- **No process outlives its test:** the test-support fixtures fail a test that leaves an engine, Daemon or handler process alive after teardown.
- **Each Product's SDK packages run against their own Product's executable**, and each implements every Client conformance case and passes the completeness checks for its own Product's catalog.
- **Every Client conformance case in `conformance/clients/` is implemented on every surface it lists**, carries its case ID, and runs in both of its surface's modes: named cases `attached` and `daemon` in TypeScript, Python and the CLI, `linked` and `daemon` in Rust, which links the engine and starts no Attached engine. A Quick-suite check fails on a missing implementation or an unknown ID.
- **For Client conformance cases, each runner writes a case log; only `goodfetch-conformance`'s writer turns it into a result document**, deriving each outcome through `goodfetch-protocol`'s constructor. A log entry never carries an outcome. A declared case missing from the log is `unproved`. Journeys write their own documents ([Journeys](#journeys)). The entry type is in `goodfetch-protocol` and its schema in the bundle; the script format's schema is `test-support/fixtures/engine-scripts/script.schema.json`. Both come from `xtask codegen`.
- **Lifecycle conformance cases** ([ADR 0078](docs/adr/0078-an-sdk-program-opens-an-explicit-engine-connection-and-closes-only-what-it-started.md)) run in every SDK: two connections at once in one process, a second `connect()` after `close()`, `closed` on a call after `close()`, nothing written to stdout or stderr by the SDK, no signal handler or event-loop patch after import or `connect()`, Ctrl+C reaching only the test's own process, a Borrowed page left open after its borrow ends, and an Attached engine's unexpected exit raising an error that carries its stderr tail.
  _Why:_ they are the failure points of the golden scenarios "Run a crawl inside a web service" and "Add an AI step to an existing Playwright program", and the lifecycle failures competitors shipped (ADR 0078).
- **Error and outcome conformance cases** ([ADR 0079](docs/adr/0079-failures-raise-grouped-typed-errors-and-expected-outcomes-are-values.md)) run in every SDK against the Scripted engine: each Error code raises its class under its group; a code the SDK does not know raises its group's class with the code kept; unknown outcome kinds, Block causes, skip reasons and Verdicts are kept, and an unknown Verdict is never `verified`; after a dropped connection an in-flight read is re-sent, while an in-flight Effect raises `UnavailableError` and is never re-sent; each SDK declares its language in `hello` and shows the target the engine rendered; and no error or call log carries a Secret canary's value. The engine's target renderer is tested once per SDK language in the Quick suite, and a Quick-suite check fails when codegen would emit a class name that shadows a built-in of any SDK language.
  _Why:_ they are the failure points of the golden scenarios "Extract a typed object with a chosen model", "Crawl two page types into a Dataset" and "Reach a goal from one instruction, with limits and a verdict", and the false successes and stringly errors competitors shipped (ADR 0079).
- **Time conformance cases** ([ADR 0080](docs/adr/0080-sdk-timeouts-nest-waits-arm-when-called-and-a-program-sets-limits-but-never-raises-them.md)) run in every SDK: an inner call ended by its outer timeout and naming it, `0` refused, the null form meaning no timeout, a response that arrives before its waiter is awaited still caught, a signal, a cancelled task or a dropped future reaching the scripted engine as `$/cancel` and stopping its work, a call stopped by another Client failing with `call_cancelled` while the caller's own cancellation never raises it, a Human pause not counted, a Limit reached ending the Run `stopped` and `incomplete`, and the sleep tombstone failing with `removed` in TypeScript and Python, and Rust's failing to compile (a compile-fail test).
  _Why:_ they are the failure points of the golden scenarios "Fill a dependent form and read the results" and "Run a crawl inside a web service", and the timing, hang and cancellation failures competitors shipped (ADR 0080).
- **Concurrency conformance cases** ([ADR 0081](docs/adr/0081-each-sdk-follows-its-languages-concurrency-model-and-shapes-events-by-meaning.md)) run in every SDK: Python's sync API inside a running ipykernel loop and from several threads at once; Python's async API under asyncio and under Trio, and a call from another loop failing at once; Ctrl+C during a sync call reaching the scripted engine as `$/cancel`; a listener's exception raising `listener_failed` once, on its value's next call; a `Subscription` and an iterator unsubscribing on close, on leaving the loop and on release; an unread lossless iterator slowing its producer; a regular expression with look-around refused at the call, and `\d` and `\w` matching as in the pattern's own language; the async Python client starting an Attached engine under asyncio's selector loop on Windows, with no event-loop policy set; and in Rust, handles checked `Clone + Send + Sync + 'static` at compile time, the crate's futures run on a single-threaded executor, each Product's `blocking` module (`goodfetch_collie::blocking`, `goodfetch_foxhound::blocking`) used inside an async runtime without panicking, and an async connection closed and dropped inside a running Tokio runtime without panicking.
  _Why:_ they are the failure points of the golden scenario "Run a crawl inside a web service", and the concurrency failures competitors shipped (ADR 0081).
- **Typed data conformance cases** ([ADR 0082](docs/adr/0082-sdk-schemas-come-from-each-languages-own-schema-types-payloads-are-typed-per-label-and-results-keep-what-the-sdk-does-not-know.md)) run in every SDK against the Scripted engine: a schema in the language's idiom and a hand-written JSON Schema both reach the engine as the same JSON Schema; a candidate a native rule rejects is retried with that rule's message; a validator that throws stops the call with `schema_validator_failed` in TypeScript and Python, while in Rust a `Deserialize` error is retried and a panic unwinds in the caller; a non-JSON value is refused with its field path and never stringified; a wrong or oversized Payload fails its enqueue with `payload_invalid`; a resume after a Payload's JSON Schema changed is refused with `payload_schema_changed` before any work, while a Payload only a changed native rule rejects fails its own request with `payload_invalid`, never retried; a native rejection outside extraction fails its call without a retry; a result field or value the SDK does not know is kept and nothing raises.
  _Why:_ they are the failure points of the golden scenarios "Extract a typed object with a chosen model", "Crawl two page types into a Dataset" and "Resume a killed crawl and retry its failures", and the schema-library breaks and stringified payloads competitors shipped (ADR 0082).
- **Settings conformance cases** ([ADR 0083](docs/adr/0083-a-program-sets-settings-in-one-typed-option-explicit-values-beat-the-environment-and-no-value-is-silently-ignored.md)) run in every SDK against the Scripted engine: a call's argument beats `settings`, and the nearest scope's `settings` beats an enclosing scope's and the project file's; an explicit `engine` beats `GOODFETCH_URL`; `timeout.run` on a Run and `timeout.connect` anywhere in `settings` fail with `setting_not_applicable`, a project file's `timeout.connect` bounds `connect()`, and one in user configuration is refused; an unknown `GOODFETCH_` variable, an empty one and an unparsable one each fail `connect()` before any call; the other Product's engine-path variable, set to a path that does not exist, is recognized and not read, so `connect()` succeeds and the TypeScript and Python SDKs start the Attached engine that their own Product's variable names (ADR 0090); an environment map passed as an option replaces the process's, which is read once and never at import; an unknown key, a wrong type, a setting no scope beneath can use (`browser.*` on a page, or set where a Borrowed browser's Session opens) and a locked or capped value each fail where the scope opens with `setting_unknown`, `setting_invalid`, `setting_not_applicable` or `setting_locked`, never clamped or ignored; a `browser.locale` set where a Session opens that conflicts with a `stealth` Identity profile or an imported login fails with `setting_locked` naming the setting that fixed it, while an inherited one yields; a capability the platform lacks fails with `capability_unavailable`; a project file sent to an Exposed Daemon that would widen a reserved allowance fails with `setting_locked`, while a Recipe's lock wins over it quietly, as on a local engine; `explainSettings()` names each value's layer and origin, and no log, error or `gf.engine` shows `GOODFETCH_TOKEN`'s value; the project file is found upward from the working directory, another is named by `project`, and `null` reads none; a misspelled key in a literal fails `tsc`, pyright and mypy, and a wrong value type for a Rust key fails to compile (a compile-fail test).
  _Why:_ they are the configuration failure points of every golden scenario, and the swallowed options, surprising precedence and import-time environment reads competitors shipped (ADR 0083).
- **Page API conformance cases** ([ADR 0085](docs/adr/0085-collies-page-api-keeps-playwrights-names-strictness-waits-values-cross-as-json-and-unanswered-requests-fall-through.md)) run in every SDK, those that need a page against the binary of the same commit on Proving ground pages (ADR 0069): an ambiguous target that becomes unique before its deadline is acted on, while one still ambiguous at the deadline raises `target_not_unique` listing every match in the SDK's language; two gestures sent at once on one page never interleave, while a read and an armed waiter run beside them, and a locator or route handler that clicks runs while the gesture it interrupted waits, without a deadlock; a locator handler that throws fails the action it interrupted with `page_handler_failed` and stays registered; a route that settles nothing falls through to the route registered before it, page routes before Session routes, and then to the network, and later routes run first; an `alert`, `confirm` or `prompt` with no handler, or with handlers that answer nothing, is dismissed, while a `beforeunload` dialog is accepted and its navigation completes; in TypeScript and Python, a second answer raises `route_already_handled` and a route handler that throws aborts its request and raises `page_handler_failed` from the call waiting on it, while in Rust a route or dialog dropped unanswered gives its default; a page function returning a DOM node or `NaN` raises `page_value_invalid` naming its path, and one reading a variable of the program raises `page_function_failed` whose fix names the argument; with `site_preferences.respect_use_signals` on and a declared Purpose, a Collie Run's own `goto` to a page whose Content Signal refuses that Purpose still loads it and records the signal in Provenance; a body read after `waitForRequestFinished` never fails; a `text=` selector or a `:has-text()` pseudo-class raises `selector_invalid`; and a CDP session is refused with `capability_unavailable` on Firefox and WebKit, and on Chromium marks the Session's results unguarded.
  _Why:_ they are the failure points of the golden scenarios "Fill a dependent form and read the results", "Crawl with HTTP and a browser only where needed" and "Add an AI step to an existing Playwright program", and the driver traps competitors shipped (ADR 0085).
- **Politeness conformance cases** ([ADR 0086](docs/adr/0086-a-crawl-obeys-robots-txt-and-sends-one-request-at-a-time-per-domain-by-default-and-pressing-harder-is-recorded.md)) run against the engine on Proving ground hosts, with short durations set: with nothing set, a crawl keeps one request in flight per domain and the delay between starts, across two runs sharing the key and counting a handler's own `goto`; it obeys a robots.txt Disallow for the `goodfetch-foxhound` group, written in any case (`GoodFetch-Foxhound` applies, as RFC 9309 matches the token case-insensitively), for `site_preferences.robots_txt_user_agent`'s group merged with it, and for `*`, start URLs included; it does not obey a group that names only `Foxhound`, which a substring match, as CPython's `urllib.robotparser` does, would wrongly apply; it reads a robots.txt answered with 404 as no rules, while one answered with 429 or 500 backs off the domain, is never cached as no rules, and with no earlier copy leaves its waiting requests skipped with `robots_txt` after their retries; it waits out a Crawl-delay of 2 s across the domain and skips a host asking for more than `site_preferences.max_crawl_delay`, set to 3 s, with `crawl_delay`; it backs off on 429, 502, 503, 504 and a refused or timed-out connection, but not on a 500 for a page; it never gates a Collie program's own `page.goto` on robots.txt or counts it in a domain's Politeness; a crawl's Run that switches hedges or `fetch.tier_race` on at a concurrency of 1 is refused with `setting_not_applicable`; each value that presses a site harder (concurrency above 1, a shorter delay, backoff off or its waits shortened, robots.txt off, sharing other than `shared`, an operator token) appears once in the Audit log with its layer, in the run's result and as one warning in the host's log, and a Record fetched against robots.txt carries the Provenance mark; and in every SDK a crawl-only key set on a Collie Run on a Foxhound engine fails with `setting_not_applicable`. The robots.txt cache rule (a copy used for 24 hours, the last good copy kept applying while robots.txt fails) is a pure function of "now", tested in-process.
  _Why:_ they are the failure points of the golden scenario "Crawl two page types into a Dataset", and the unsafe defaults competitors shipped (ADR 0086).
- **The navigation gate** ([ADR 0087](docs/adr/0087-each-product-is-an-executable-crate-that-links-only-its-crates-and-has-its-own-size-budget.md)) is tested in the same cases: a handler's click that navigates, a redirect by the page's own JavaScript, a fetch through its page's `request`, an AI verb that navigates and a navigation in a popup the handler's page opens are counted and gated like its `goto`, each held until its response arrives (a browser-tier page until its load event); the load of the page handed to the handler takes no second permit; a page a program opens is never counted in per-domain Politeness; and on a Collie engine a key only Foxhound honours fails with `not_in_product`.
  _Why:_ a handler's own navigation is how a crawl would press a site past its limits unnoticed (ADR 0086).
- **A handler's page past its work item's lease sends nothing more** (ADRs 0003, 0087): once the lease ends, a command on that page or a popup it opened fails before reaching the browser, and a raw CDP attach bound to it ends with `attach_ended`; a page a program opens is fenced by the run epoch alone.
  _Why:_ the lease check reaches Collie's page only through the slot Foxhound sets, so a missing slot would drop the fence silently.
- **Session ownership and proxy limit conformance cases** ([ADR 0088](docs/adr/0088-collie-carries-every-capability-except-those-only-many-pages-need.md)) run against the engine on Proving ground hosts. A 403, a 429 and a Challenge met by a Session the program or an Agent opened, with `stealth` allowed, never retire it or open another Session, on a Saved identity, a Profile directory used in place and a Borrowed browser alike. The Challenge goes to the Challenge solver when `solve_challenges` is on, then to a Human pause when a human channel is configured, otherwise to `challenge_unsolved`; the 403 and the 429 are reported on the call that met them, with no Human pause; and in every SDK the program's Session and page stay usable afterwards. Once the Challenge solver has cleared a Challenge on a domain, the program's next Challenge there starts at the solver, with no `stealth` Session opened. The same 403, 429 and Challenge on a Session of a crawl's Session pool retire it and open its replacement, or, on a Borrowed browser, raise a Human pause. A 429 from a Collie program's proxy itself is a proxy error, never a Block cause, and lowers that proxy's ceiling for the user's other runs; two Collie runs whose `politeness.proxy.concurrency` is 1 take turns through that proxy instead of failing; and in every SDK a Collie Run accepts `politeness.proxy.concurrency`, `politeness.proxy.rate` and `politeness.sharing`.
  _Why:_ a replaced Session would lose the login of the golden scenario "Log in once with a person, reuse the login headless" and show the site a different browser (its failure point 6), and an Agent's page in "Reach a goal from one instruction, with limits and a verdict" would lose its state; and runs sharing a proxy plan would fail on the vendor's refusals instead of taking turns (ADR 0088).
- **Lending conformance cases** ([ADR 0089](docs/adr/0089-a-page-is-lent-through-an-endpoint-that-shows-only-its-session-and-collie-joins-other-crawlers-by-borrowing.md)) run each supported library (Playwright in TypeScript and Python, Puppeteer, Browser Use) through its helper against the binary of the same commit: at its newest pinned release in the Pre-merge suite when the SDK or the endpoint is touched, and at both ends of its supported range in the Release suite. They check that the library sees only the lent page, the pages it creates and the targets opened from either, never another Session or a page the program opens during the attach; a second lend on the same Session is refused with `session_reserved`; `Browser.close` ends only the attach and the browser lives; a new browser context, an extension load, ignoring certificate errors and a browser-level command the table does not class are refused with `cdp_command_refused`, and a file read by path on a Daemon with accounts; a download stays the Run's; device emulation and permissions are back after the attach ends; the lease's end raises `attach_ended` in the helper and a reconnect with the old token is refused; in a crawl handler, a library's navigation on the lent page and on a page it creates passes ADR 0087's navigation gate once, and a navigation robots.txt disallows is refused; and lending a Firefox or WebKit page raises `capability_unavailable`.
  _Why:_ they are the failures the driver interop research measured on a page another program owns (ADR 0089).
- **The crawler guides' examples** ([ADR 0089](docs/adr/0089-a-page-is-lent-through-an-endpoint-that-shows-only-its-session-and-collie-joins-other-crawlers-by-borrowing.md)) (Crawlee for JavaScript and Python, Scrapy with scrapy-playwright, on Chromium) run as [docs code samples](#docs-code-samples) at pinned crawler versions, each borrowing pages from two browser contexts in one Run and checking one Session per context.
- **AI verb conformance cases** ([ADR 0091](docs/adr/0091-collies-ai-verbs-live-on-the-page-replay-from-a-plan-file-the-run-names-and-expect-runs-a-success-check.md)) run in every SDK. Against the Scripted engine: each code raises its class under its group with its details; `StepResult` carries `output`, `step` and `usage`, and an unknown step source is kept; `observe` with no match returns an empty list; a placeholder nothing fills, an unused variable or a name that is both a variable and a bound Secret placeholder fails with `variables_mismatch` before any request; the Plan file is read and written on the program's side, as with a Daemon. Against the binary of the same commit with the testing kit's Scripted model: a step the Plan holds replays with no model call and says `plan`; a missing step compiles once and says `model`, and two calls for the same entry at once compile it once; a drifted step is repaired once and says `repair` with its drift, while a Drift after that Repair fails with `plan_drift`; under `plan.frozen` no model is called and nothing is written, a missing step fails with `plan_step_missing`, and `extract` with `mode: "model"` fails with `setting_not_applicable`; a new Plan version is written only after a `verified` Run, kept pending otherwise, and written once accepted by the next Run that names the file; two Runs that change different entries of one file both keep theirs, and an entry both changed is kept pending, never overwritten; a variable's value never appears in a request the Scripted model recorded, in the Plan file or in a log; `expect` passes when its condition holds and fails with `success_check_failed` once `timeout.wait` passes, a Run whose last `act` or `observe` call no passing Success check follows is `needs-review`, and a Success check evaluated before an `act` does not make it `verified`; a required field not on the page fails with `extraction_incomplete` carrying the fields found, and a model that never answers validly with `extraction_invalid`; an `act` whose Effect was sent before a failure raises `effect_uncertain` and is never re-sent; an `ai` aria snapshot past `observation.max_size` collapses and says so; `getByRef` on a removed element fails with `target_not_found` at once; with `site_preferences.respect_use_signals` on, a compile on a page whose AI-use signal refuses the declared Purpose fails with `site_preferences_refused` (`use_signals`) before any model request, while a replayed step runs. In Rust, typed details are read through `Error::details()`, and Python raises no `AssertionError`.
  _Why:_ they are the failure points of the golden scenarios "Extract a typed object with a chosen model", "Repeat AI steps with no model call", "Extract with AI inside a crawl, learning once per page type", "Add an AI step to an existing Playwright program" and "Reach a goal from one instruction, with limits and a verdict", and the server-only caches, false successes and token-heavy observations competitors shipped (ADR 0091).
- **Crawl conformance cases** ([ADR 0093](docs/adr/0093-a-crawl-declares-its-labels-and-one-handler-each-and-a-handler-reads-collies-page-on-every-fetch-tier.md)) run in every SDK against the Scripted engine, and against the engine on Proving ground hosts where a tier or a site is involved. A second Label of one name, or a second handler, fails at once with `label_duplicate`; a Label left without a handler fails the start with `label_unhandled`; an enqueue to another crawl's Label, and a resume whose pending requests name an undeclared Label, fail with `label_unknown` before any work; a missing or wrong Payload on `enqueue`, `seed` or `addSeed` fails `tsc`, pyright and mypy. `skip()` ends the request `skipped` with the reason `handler` and its message, and `retry()` ends the attempt with `retry_requested`, counted against `crawl.retry.max`, even when the handler swallowed either; every later call on that context fails with `attempt_ended`, and in Python `except Exception:` does not catch it. A handler raising its own exception ends `failed` with `handler_failed`, carrying the exception's type, message and stack, and is not retried unless `crawl.retry.handler_failed` is on, when it is retried `crawl.retry.max` times; a retryable `GoodfetchError` raised inside, such as a navigation timeout, keeps its code and is retried. A handler process killed mid-attempt, or an Attached engine that died with it and is resumed, leaves a `handler_lost` attempt that counts as a retry, while the program's own `close()` or a dropped Rust `serve` future counts none, a Rust handler's panic ends `handler_failed`, and a Hook's retry draws on the same `crawl.retry.max`. Enqueues, seeds and Records an attempt staged are discarded when it fails, retries or escalates. A 5xx or a 408 is retried and then fails `response_unavailable`; a 404 fails at once with `response_not_ok`, unless `crawl.handle_statuses` lists it, when the handler gets it. On the Proving ground's HTTP and JavaScript-rendered pages with `fetch.tier: auto`, one handler reads both: a wait for content the static document lacks, a click, and a page failing its `expectedPage` each escalate the request to the browser, none counting as a retry, and a `page.request` never escalates; under a forced `http` tier the same click fails with `browser_required`. Two `enqueue` calls to one URL with two POST bodies add two requests, and the same body twice is a `duplicate`; an `enqueueLinks` Payload function gives each link its own Payload and skips a link it returns `null` for; a `sitemap(url, rules)` seed, read from robots.txt, follows a nested index, gives each entry the first matching rule's Label, and reports the entries matching none, under `no_rule`, in the Run's report. A link outside `crawl.scope.strategy`, or beyond `crawl.max_depth` or `crawl.max_requests`, is answered "not added" with its reason, as is a seed the domain policy refuses; a redirect leaving the scope ends `skipped` with `out_of_scope`; a handler's `goto` that robots.txt disallows, or, with `site_preferences.respect_use_signals` on, that an AI-use signal refuses for the Run's Purpose, before the request or in its response's `Content-Usage` header, fails with `site_preferences_refused` and its reason (a refusal from the response leaving the page blank), and escaping the handler it ends the request `skipped` with `robots_txt` or `use_signals`, while a Collie program's own `goto` to the same page is not refused. A Label takes `locator.test_id_attribute` and `session.page_check` without a pool of its own, gets Sessions apart from the Run's with its own `session.source`, and refuses `politeness.*` and `browser.*` with `setting_not_applicable`, as a Collie Run on a Foxhound engine refuses `fetch.*` (a Collie engine answers `not_in_product`). A handler's `goto` over the HTTP tier is fetched over it through the gate; a page's own redirect that robots.txt disallows is aborted and reported as a `requestfailed` event, failing only a waiter armed for it; a handler waiting for a permit on a domain with a long delay is not timed out; and a TypeScript `enqueue` left unawaited after the handler returned fails with `attempt_ended`.
  _Why:_ they are the failure points of the golden scenarios "Crawl two page types into a Dataset", "Crawl with HTTP and a browser only where needed" and "Extract with AI inside a crawl, learning once per page type", and the tier rewrites, faked skips and blind retries competitors shipped (ADR 0093).
- **Sessions, Human pause and secrets conformance cases** ([ADR 0092](docs/adr/0092-sessions-open-from-their-source-page-pause-waits-for-a-person-and-a-secret-is-a-placeholder-bound-to-domains.md)) run in every SDK against the Scripted engine: `run.newPage()` twice shares one default Session, reached from each page, and after it is closed `run.newPage()` fails with `closed`; `source` on `newSession` beats `session.source` from the Run, `session.source` in a Session's `settings` fails `tsc`, pyright and mypy, and in Rust is refused with `setting_not_applicable`; `saveIdentity`, the cookie calls and `gf.identities` send their operations and map their results, a cookie's expiry and a Saved identity's times arriving as the language's point-in-time type (`Date`, an aware `datetime` in UTC, `SystemTime`); `page.pause()` sends its reason, `until` and timeout, returns who ended it and how long it waited, throws `human_pause_declined` on a decline, throws `deadline_exceeded` naming `timeout.human_pause`, and sends `$/cancel` when the call is cancelled; the `login` function (a closure in Rust) runs on the program's side when the engine asks; `blocked` arrives as a `PageError` carrying its Block cause, URL and status, an unknown cause kept as received; `page.localStorage` and `sessionStorage` send their operations; a Secret placeholder outside the positions that take one is refused before sending with `page_value_invalid`; `resolveSecret` (Rust: `secret_requests()`) answers a `caller:` request, and none or a throw maps to `secret_unavailable`; each new Error code raises its class; `keyboard.type(secret(…))` fails `tsc`, pyright and mypy, and a literal with no provider where a Secret reference is expected fails `tsc`; in Rust, a placeholder passed to `evaluate` fails to compile (a compile-fail test); cookie and storage operations appear on no CLI command, MCP tool, skill, Framework adapter or Code mode global (a catalog completeness test).
  _Why:_ they are the SDK failure points of the golden scenario "Log in once with a person, reuse the login headless" (ADR 0092).
- **Sessions, Human pause and secrets engine cases** ([ADR 0092](docs/adr/0092-sessions-open-from-their-source-page-pause-waits-for-a-person-and-a-secret-is-a-placeholder-bound-to-domains.md)) run against the engine on Proving ground hosts, with short durations set: a Saved identity kept by `saveIdentity` is reused by a later Session, and `saveIdentity` on a Borrowed browser fails with `session_not_savable`; cookies outside a bound source's `domains` are refused; a pause stops every enclosing timeout, holds the page's later Gestures while reads run, ends when `until` holds or on "Done" (refused while `until` does not hold) or "Hand back", ends for a Live view grant holder only through `until`, ends past `timeout.human_pause`, and leaves Usage counting toward Limits; a page the Page check judges a login page runs the login step, skips the Human pause when `session.recovery.human_pause` is off, and fails with `logged_out` listing each step; on a Session the program opened, a rate limit or IP block fails the `goto()` that met it, or a `click()` that waits for the navigation it started, or else the page's next call after a navigation no call waits for (a script's redirect), with `blocked` naming the cause, and the Session is neither replaced nor closed, while a `page.request` answered with a rate limit returns its response; with `session.recovery.human_pause` off, a Challenge the solver cannot pass fails with `challenge_unsolved` without a Human pause; a Secret placeholder typed by `fill` or sent in a header resolves only when the field's frame or the request is on its bound domains (`secret_domain_not_allowed` otherwise, a payment frame on an allowed page included), an unbound one fails with `secret_unbound`, and a missing value with `secret_unavailable`, as does a `code_source` with no code while the pause is off; a header secret sent by `page.request` is invisible to page scripts and a Service Worker, and a redirect to a domain outside the binding drops it (a header and a resent JSON body) with `secret_domain_not_allowed`; a resolved value, and the Basic `Authorization` and `Proxy-Authorization` headers built from credentials, never reach a `request`, `response`, `console`, `pageerror`, `dialog` or WebSocket event, where `%name%` stands instead (the setting's key, `%session.http_credentials%` or `%proxy.password%`, for those headers), except a one-time code, which is masked where it was typed and in what the engine keeps; a placeholder typed into or sent from a Borrowed browser other code drives, or a page with a raw CDP session, fails with `secret_input_refused`, as does an HTTP authentication challenge there or on a Lent page; `secrets.*`, `session.http_credentials`, `proxy.server` and the proxy's credentials set by a Client acting for a model fail with `setting_locked`, and set by a Recipe are refused, while a Recipe's pool kind is kept; a `saveIdentity` from a Client acting for a model or a Recipe run waits for a person's approval; `browser.permissions` with a clipboard permission in a headful window on the person's desktop fails with `setting_not_applicable`; storage outside a bound source's `domains` is refused; a Saved identity from a login bound to its machine stays bound, and its export bundle used on another machine fails with `session_bound_to_machine`; a value that is not a Secret reference is refused with `setting_invalid`; `session.http_credentials` answer only a listed domain's challenge.
  _Why:_ they are the engine failure points of the golden scenario "Log in once with a person, reuse the login headless", and the leaked secrets, unbounded pauses and lost logins competitors shipped (ADR 0092).
- **Agent conformance cases** ([ADR 0100](docs/adr/0100-collies-agent-is-one-bounded-call-on-a-page-judged-by-the-programs-success-check-and-replayed-whole-from-the-plan.md)) run in every SDK against the Scripted engine, with the Scripted model behind it (ADR 0065): a task whose model reports done while the program's Success check fails gets the failure back and goes on, and ends `rejected` if the Success check never passes; a task that reaches `maxSteps`, the stall rule, `timeout.agent` or a Limit returns `ended: stopped` with that reason and its Grounded partial answer, with `reported` absent when the model claimed nothing, and only a Limit stops the Run; an enclosing handler's timeout fails it with `deadline_exceeded`; a task another Client stops returns `abort`, while the caller's own cancellation keeps its language's form; `steps()` called after the task began still starts at step 1 and loses none, and raises the task's error when it failed; an answer whose value appears on no observed page and in no tool's result reads `needs-review`; a replayed task makes no model call; a Drift, or a failed Success check after the replay, hands over to the loop from that step; under `plan.frozen` a Drift raises `plan_drift`, a missing entry `plan_step_missing`, and a failed Success check after the replay finishes the task `rejected`; with no model configured a missing entry raises `model_not_configured`; an Agent tool's gesture on the task's page runs while the task waits for it; a tool that raises a `GoodfetchError` ends the task with that code, any other exception with `agent_tool_failed`; two tools with one name, or one named like a built-in action, fail with `agent_tool_invalid` before any step; a `PageError` from the Success check function goes back to the model, another `GoodfetchError` ends the task with its code, and any other exception raises `success_check_threw`; a replay checks before an Effect only the targets the recorded run saw before it, so a target that appears after an Effect on the same page is no Drift; a sent Effect left uncertain ends the task with `effect_uncertain`, and a declined confirmation with `effect_not_confirmed`; a navigation outside `agent.domains` is refused and given back to the model, while the page's own subresources load; `session.recovery.agent` off, or no model and no login Plan, skips the AI agent login step, and `logged_out` names why; an error raised after steps carries `agentTask` with the steps, the partial answer and any tool left uncertain; with `agent.human_pause` on, a person's answer, a decline and an expired pause each go back to the model; a tool with a `category` asks for confirmation unless the run allows it, and always for payment and accepting terms; a replayed task's `reported` is absent and its `ended` is `finished`; the tabs a task opened are closed, except the page it ended on, and on a Borrowed browser only released.
  _Why:_ they are the failure points of the golden scenario "Reach a goal from one instruction, with limits and a verdict", and the false successes, silent loops, hidden step caps and off-site navigation competitors shipped (ADR 0100).
- **Dataset and Record conformance cases** ([ADR 0103](docs/adr/0103-a-handler-pushes-records-staged-with-its-attempt-into-datasets-its-crawl-declares-and-a-reader-never-slows-the-crawl.md)) run in every SDK against the Scripted engine, and against the engine on Proving ground hosts where a commit, a Sink or a remote engine is involved; on Collie's executable for the Key-value store outside a handler and a Collie engine's `sinks.*` cases, and on Foxhound's for the rest (ADR 0094):
  - **Declaring.** A second Dataset of one name fails at once with `dataset_duplicate`, and a key on a field the schema does not require with `dataset_key_invalid`. A resume whose crawl no longer declares a Dataset holding Records fails with `dataset_unknown`, and one whose key changed with `dataset_key_invalid`, before any work; one whose schema changed resumes.
  - **Pushing.** A Record missing a field of its Dataset's schema fails `tsc`, pyright and mypy. A push to another crawl's Dataset fails with `dataset_unknown`. A Record that fails its schema natively or in the engine, is not JSON, or lacks a key field of a Dataset with a key and no schema fails `push` with `record_invalid` and is not staged. Caught, the attempt's other Records commit; escaping the handler, the request ends `failed` and is not retried.
  - **Committing once.** Records, enqueues and Key-value store writes through `ctx.run` that an attempt staged are discarded when its handler fails, or when it retries, escalates, skips, times out, loses its lease or ends with the Run's close. A `push` left unawaited until after the attempt ended fails with `attempt_ended`. A request retried twice commits its Records once, numbered with no gaps. A sample comparison commits only the Records of the browser attempt whose blocking setting the route keeps, and a Label whose browser attempt enqueues links the `http` attempt does not find is not moved to `http`. A crash in mid-commit, and a zombie copy with a stale run epoch, commit no Record twice. A Key-value store write through the program's own Run value inside a handler lands even when the attempt fails.
  - **Hooks.** A "Record before commit" Hook sees only the committing attempt's Records. Its drop or reject leaves the request `succeeded`, counted per Dataset on its outcome. A transformed Record that no longer fits its schema is rejected, and `rejectedRecords()` reads it back as JSON with its number. A Hook that fails discards the attempt's staged outputs.
  - **Reading.** `records()` called after Records have committed starts at the first, `after` resumes past a number, and the iteration ends when the Run ends. With a `records()` iterator open and never read, the crawl reaches `finished` with every request committed, and no commit waits on the reader. Through a handler's `ctx.run`, `records()` reads what has committed and ends. A Record its native parse rejects ends the loop with `record_invalid` and its number.
  - **Exports, the Key-value store and Sinks.** `export()` writes the Records committed so far, and `saveAs` puts the file on the program's machine, against a Daemon too. `get` on an absent key answers null; `tsc` rejects `set(key, null)`, and in Python and Rust it fails with `value_invalid`, as bytes read with `get` do. Through `ctx.run`, `get` sees the attempt's own staged `set`, another attempt's stays invisible until it commits, and of two commits to one key the later wins. A Sink set on the Run naming an undeclared Dataset, in a keyed mode with no key of its own while one of its Datasets has none, or with a key naming a field one of its Datasets' schemas does not require, is refused at start with `setting_invalid`; one inherited from the connection is inactive for a crawl that declares none of its Datasets; `sinks.*` set on a Label, or on a Collie Run on a Foxhound engine, fails with `setting_not_applicable`, and given to a Run or a connection on a Collie engine with `not_in_product`. A Sink set on a Foxhound connection is inactive for the Collie Runs it opens. A Collie program starts in a project whose file holds a Sink for its crawls, and `config explain` lists that Sink as Foxhound's. Two Datasets delivered to one Sink each keep every Record, though both number from 1.
  _Why:_ they are the failure points of the golden scenarios "Crawl two page types into a Dataset", "Resume a killed crawl and retry its failures" and "Run a crawl inside a web service", and the partial and duplicate results competitors shipped (ADR 0103).
- **Completeness tests** check that every catalog operation, Error code and event has its generated counterpart in each SDK.
- **Generated code is type-checked** by `tsc` with `skipLibCheck: false`, pyright strict and mypy `--strict`.
- **Mapping and decoding are property-tested** (fast-check, hypothesis): user-data values with any keys, `__proto__` included, come back unchanged.
- **Type tests:**
  - TypeScript: `*.test-d.ts` files with `expect-type`'s `expectTypeOf` and `@ts-expect-error`, compiled by `tsc --noEmit` against the built `.d.ts`. Not Vitest's `typecheck` mode, tsd or TSTyche.
  - Python: `tests/types/` files with `typing.assert_type` and `# type: ignore[code]`, under pyright strict with `reportUnnecessaryTypeIgnoreComment = "error"` and under mypy `--strict`, whose `--warn-unused-ignores` reports an unused ignore.
- **Python:**
  - the test-support plugin's `anyio_backend` lists `asyncio` and `trio` explicitly, and a missing trio fails the session;
  - every Client conformance case runs on asyncio, on trio and through the sync API;
  - free-threaded builds run the whole suite with `PYTHON_GIL=0`, and fail if the GIL is on; the sync API's unit tests also run under pytest-run-parallel. Every Linux platform in the Release suite; macOS and Windows in a nightly-only job.
- **TypeScript runtimes:** Vitest runs on Node with the `forks` pool. The `smoke/*.mjs` scripts (only `node:assert`) each run spawned by a Vitest test:
  - on Bun and Deno 2, with an Attached engine and with a Daemon, covering the runtime-dependent Client conformance cases, whose IDs the script declares;
  - on `workerd` (Miniflare) and Vercel's edge runtime, against a Daemon only, since they cannot start a process. Deno 2's `daemon` run stands for Deno Deploy (ADR 0049).
- **Installed packages:**
  - npm packages install from Verdaccio with no uplink, with npm, pnpm, Yarn, Bun and Deno, into a path with a space and a non-ASCII character, and the smoke scripts check the installed version; `--omit=optional` must fail at first use with Goodfetch's error;
  - each install keeps its manager's release-age gate on and passes it through ADR 0030's exclusion snippet, read from the source the release notes use; one run per manager without the snippet must not install the packed version (uv's snippet is not tested: `--no-index` has no gate);
  - wheels install into a fresh environment (`uv pip install --no-index --find-links`, and `pip`) and run `tests/integration` unchanged, on glibc and Alpine;
  - each Product's crate builds a temporary consumer from `cargo package`'s output;
  - one environment per package manager installs both Products' packages, and both executables run there.
- **Models are scripted only through the testing kit's Scripted model** (`goodfetch_collie::testing`, `@goodfetch/collie/testing`, `goodfetch_collie.testing`, and Foxhound's `goodfetch_foxhound::testing`, `@goodfetch/foxhound/testing` and `goodfetch_foxhound.testing`). The kit's test engine helper is an Attached engine in TypeScript and Python and the linked engine in Rust.

### CLI

- **The binary runs under assert_cmd and is snapshotted with insta**, through `goodfetch-test-support`'s helper. One snapshot holds stdout, stderr and the exit code.
- **Every `--json` test validates stdout against the command's schema** from the bundle, checks that stdout holds nothing else, and snapshots it.
  _Why:_ `--json` is under semver (ADR 0030), and only a schema puts it in the breakage check.
- **Warnings, hints, deprecations and the age notice never reach stdout.**
- **Every exit code in `goodfetch-protocol`'s table has a test through the binary**, one rstest named case per exit code, in `tests/it`. An in-process unit test checks that the table maps every Error code.
- **Test a command both executables carry through Collie's.** Foxhound's `--help` snapshot and the containment checks of [Which Product's executable a test runs](#which-products-executable-a-test-runs) show it unchanged there.
- **Human output is snapshotted under `NO_COLOR=1`, a fixed `COLUMNS`, `TERM=dumb` and a fixed locale.** The `--help` tree is one snapshot rendered in-process from clap's `Command`; one `tests/it` case checks the binary prints the same. A unit test calls clap's `Command::debug_assert`.
- **Terminal-only behaviour is tested in a pseudo-terminal with expectrl:** colour, progress, and person-only prompts, which must refuse when stdin is not a terminal.
- **Interrupts are tested through the binary:** a running Crawl sent `SIGINT` (Unix) or `CTRL_BREAK_EVENT` (Windows) checkpoints and exits with its exit code; a second interrupt stops it at once.
- **A command that runs the engine is tested in both modes**, as rstest cases `attached` and `daemon`, against one snapshot.
- **Client faults** (a refused protocol version, a dead PID, a dropped connection) come from the Scripted engine run as a Daemon.
- **What only Foxhound carries is refused through Collie's executable,** table-driven from the generated list of Foxhound's names: each operation over the protocol, each setting key as a `settings` value, each CLI command with its exit code, and enabling each Plugin kind and Hook stage answer `not_in_product`, with one message snapshot per kind. A configuration file holding a Foxhound key loads, and `config explain` lists it as Foxhound's (ADR 0087).
- **A Recipe is held to the Product its manifest names, on both engines** (ADR 0096). `init` writes the Product of the executable that ran it. Collie refuses a Recipe that names Foxhound with `not_in_product` before its run starts. A Foxhound-only operation, Plugin kind or Hook stage from a Recipe that names Collie answers `not_in_product` on both engines, its `fix` saying to name Foxhound in the manifest, and on a person's own Collie engine also to run it with Foxhound. On Foxhound's engine, the same call on that run from a Client answers the same, the Client's connection and other calls still succeed, and a crawl setting key on that run answers `setting_not_applicable`.

### Where they run

| Suite | SDKs | CLI |
|---|---|---|
| Quick | unit tests, type tests, completeness and property tests, script validation, the case-list check | in-process unit tests, `--help` rendered in-process, `debug_assert`, the exit-code table maps every Error code |
| Pre-merge | touched SDKs on Linux with the newest Node LTS and CPython: integration, Client conformance, Scripted engine process, one installed-package run | every `tests/it` case, both modes: exit codes and `--help` through the binary, `--json`, pseudo-terminal, interrupts |
| Release | every Node line, Bun, Deno and CPython on Linux x64, free-threaded builds on every Linux platform; oldest and newest Node and CPython on every other platform; every install path on every OS. Free-threaded builds on macOS and Windows: a nightly-only job | everything, on every platform |

## MCP, framework adapters and Integrations

Decided in [ADR 0070](docs/adr/0070-mcp-adapters-and-integrations-are-tested-through-their-platforms-own-clients-against-a-real-engine.md).

### Every surface

- **Drive each surface with its platform's own client, against a real engine or Daemon serving the local Proving ground.** No mocking library or hand-written fake of Goodfetch's API: no `nock`, fake HTTP API or stub listener.
  _Why:_ a fake of the engine passes where the real one fails, and misses the token, `Host` and `Origin` checks.
- **For Goodfetch's side, use only the doubles ADRs 0062 and 0069 define:** the in-memory adapter of the protocol in Quick-suite unit tests, and the Scripted engine for faults a real engine can't produce on demand.
- **Stand in for the platform in this order:** its real runtime run locally, its official test harness, a written adapter of its context.
- **Keep a Quick check over everything `xtask codegen` generates:** no person-only operation or setting, and no `page.lend` ([ADR 0089](docs/adr/0089-a-page-is-lent-through-an-endpoint-that-shows-only-its-session-and-collie-joins-other-crawlers-by-borrowing.md)), on any MCP profile, Framework adapter, Integration or skill, and each surface's operations map to catalog operations of the same class.
- **Assert outcomes through the oracle and Error codes**, never a model's wording or a platform's message text.

### MCP server

- **Test the server with `rmcp`'s client:** in-memory in its crate, over stdio and Streamable HTTP in the binary's `tests/it`. MCP Journey variants run there too, on nextest.
- **Read stdio stdout with the strict framing reader**, logging at `trace`. Any line that is not a JSON-RPC message fails the test.
- **Test two Streamable HTTP sessions at once:** each keeps its own Limits and its own cancellation.
- **Run the conformance suite at its pinned version, once per supported revision**, with `--expected-failures conformance/mcp/expected-failures/<revision>.yaml`:
  - `--requirements <revision>` for a revision with a frozen requirement set (today `2025-11-25` and `2026-07-28`), `--spec-version <revision>` for an older one;
  - against the shipped binary, whose expected-failures file lists only scenarios that need fixture tools or resources, each with a reason (a Quick check refuses other entries);
  - against the fixture server, with an empty expected-failures file;
  - once more for the extensions the server implements, each scenario by name with `--scenario` and without `--spec-version`, against `conformance/mcp/expected-failures/extensions.yaml`, since `--requirements` doesn't score extensions. These run at `2026-07-28` only; test an extension the server supports under an older revision with `rmcp` in `tests/it`.
- **Change the suite's version only in a pull request that updates the expected-failures files.**
- **Run the pinned Inspector CLI with `--format json --stored-auth-only`:** boot smoke of every packaged artifact over stdio and Streamable HTTP (the OCI image on Linux only), `--strict` on every profile's `tools/list`, and `--verify --require-digests` on skills once the server serves them.
- **Cover every supported revision in the client matrix:** TypeScript SDK v1 and v2 clients, the Python SDK and `rmcp`, pinned, over stdio and Streamable HTTP, plus the negotiation cases.
- **Add a revision to the supported list only with its expected-failures file and its matrix cells, in the same pull request.**

### Framework adapters

- **Drive each tool through the framework's own agent loop with its own scripted model**, never by calling the tool's `execute` alone: `MockLanguageModelV4` for the Vercel AI SDK and Mastra, `FakeListChatModel` or `FakeStreamingChatModel` for LangChain.js, `GenericFakeChatModel` through test support's `bind_tools` subclass for Python LangChain.
- **Script a read, an Effect and a failing call** in each adapter's tests.
- **Run `ToolsUnitTests` on every generated Python LangChain tool**, and `ToolsIntegrationTests` on one tool per operation class and every tool a Journey uses, from a pinned `langchain-tests`.
- **Test the lowest and highest version of each declared peer range, and one of each major inside it**, in the Release suite. Pin the lowest in a test package (pnpm's `lowest-direct` never lowers peers); use uv's `--resolution lowest-direct` in Python.
- **Give each framework that connects only through MCP a model-free test** of its docs example: its own MCP client connects, lists tools and calls one tool.
- **Check Dataset loaders against the oracle's records**, including metadata and lazy streaming.

### Integrations

- **Start the Daemon through `goodfetch-test-env`** with its HTTP listener, a token and port 0.
- **Cover, for each Integration:** starting a run and a Recipe, fetching Records, one trigger per record range, no repeated trigger after a resumed run, a refused token, a refused `hello` version, and every Error code it maps.
- **Produce the refused `hello` and the Error codes the Proving ground can't cause with the Scripted engine run as a Daemon with an HTTP listener**, one script per case; test every other case against the real Daemon.
- **Run each platform's offline validator in the Quick suite**, and the Integration in its platform's local runtime or official harness in the Pre-merge suite:

| Integration | Quick | Pre-merge |
|---|---|---|
| n8n | `n8n-node lint` (strict); verification rules | pinned n8n image, `n8n execute --id` |
| Zapier | `zapier-platform validate --without-style` | `createAppTester`, real HTTP |
| Make | Make Apps Editor JSON schemas; module-to-operation map | none (Live check) |
| Pipedream | Pipedream ESLint rules, build, key checks | `run` with the `$` adapter |
| Activepieces | lint, build | `createMockActionContext`; pinned Activepieces image |
| Power Automate | `ConnectorPackageValidator.ps1`; OpenAPI 2.0 validation | every operation through an OpenAPI 2.0 client |
| Airbyte source | offline QA checks; manifest validation | CDK standard suite; incremental reads from saved state |

### Journey variants

- **Run `adapter-<name>` and `integration-<name>` variants in the package's own runner, on Linux:** Vitest in `tests/journeys/J-NNN.test.ts` for TypeScript adapters and the n8n, Zapier, Pipedream, Activepieces and Power Automate Integrations; pytest in `tests/journeys/test_jNNN.py` for Python adapters, the LlamaIndex reader and the Airbyte source. Make has no automated variant.
- **Run `http-listener` variants on nextest** in the binary crate's `tests/it/journeys`, against a Daemon's HTTP listener with a token and port 0.

### Live checks and manual tests

- **Run unpinned releases only as Live checks:** the newest conformance suite, Inspector and MCP SDK clients, and each framework's `latest` and prerelease tags, nightly; the n8n scanner after each publish.
- **Keep real platform accounts out of every suite.** They run weekly and at release candidates, on free developer plans only, through a Cloudflare Quick Tunnel (`cloudflared tunnel --url`, a new `trycloudflare.com` URL each run, no account or domain), and only once the owner gives the go-ahead at M4, because the job exposes a Daemon (ADR 0076); a repository variable keeps the job off until then. Paid accounts are declined while there is no revenue.
- **Test by hand at each release candidate that changes them:** Claude Desktop, ChatGPT, VS Code and Cursor connecting to the MCP server; `paconn validate`; each platform's listing submission.

## Performance budgets

Decided in [ADR 0071](docs/adr/0071-performance-budgets-are-counted-per-change-and-time-is-measured-on-the-reference-host.md).

### What gates a change

- **Only counted figures fail a change:** instructions and heap metrics per benchmark, protocol and render counts per scenario, and bundle bytes. No check on a GitHub-hosted runner compares a duration with a threshold.
  _Why:_ shared runners vary by about 2.7% in wall time, and instruction counts of one binary on one machine repeat exactly.
- **No test asserts elapsed time, a rate or a duration.** Time is measured only by the Performance series and the benchmark, both Live checks.

### Benchmarks

- **Benchmarks live in the crate's `benches/`:** `scenarios.rs` holds the scenario functions, `instructions.rs` runs them under gungraun's Callgrind with mimalloc as the global allocator, `heap.rs` under gungraun's DHAT with the system allocator, and `wall.rs` under criterion for the Performance series. Scenarios that cross crates (startup, a protocol round trip) live in the binary crate.
  _Why:_ DHAT counts allocations by replacing `malloc`, which mimalloc never calls.
- **A benchmark measures one pinned scenario:** inputs from a literal, a fixture or a seed; no network, no sleep, no work whose amount depends on timing.
- **A bench target builds what it needs the way tests do:** a current-thread runtime, `Clock::paused` and `Entropy::seeded` through `goodfetch-test-support`'s helpers, and the `tasks` module's inline blocking policy, so no thread waits on another. Its root opts in once with `#![expect(clippy::disallowed_methods, reason = "benchmark harness, ADR 0071")]`.
- **A new or changed benchmark runs 20 times on one build in its pull request.** Every gated figure must spread less than 0.5%, or the benchmark is fixed before it gates.
- **Changing what a benchmark measures gives it a new ID.** A benchmark with no base is reported, never gated.
- **Benchmarks build with the `bench` profile** (release settings, thin LTO, `debug = "line-tables-only"`) on the Linux gnu targets.

### The comparison with the base

- **`xtask perf` measures the base and the head in the same job, on the same runner:** the merge base on a pull request, the group's base in the merge queue. Never a stored figure from another machine.
- **The gate trips at +5%** on instructions, total allocations, total bytes or bytes at the global maximum of any benchmark, and at **+5 kB brotli** on any size-limit entry.
- **A tripped pull request passes only with a "Performance change" section** in its body, listing each tripped figure with its measured change and reason, accepted explicitly by the fresh review on the exact head.
- **Run `just perf` before pushing a change to code a benchmark covers.** It uses CI's thresholds; off Linux it runs in the CI image through a container engine, or lists itself as skipped.

### Counts as snapshots

- **Protocol counts are snapshots:** for each named scenario of a surface, the requests sent, the answers and events received, and their bytes after the shared redactions. Rust against the real binary in `tests/it`; TypeScript and Python against the Scripted engine of ADR 0069 in unit tests.
- **MCP scenarios snapshot the bytes of `tools/list` and of each tool result.**
  _Why:_ for an agent, bytes are context.
- **The Python SDK snapshots the modules `import goodfetch` loads.**
- **Console render budgets are snapshots in component tests:** React commits per action inside `<Profiler>`, and DOM elements rendered for a 10,000-row list. Never render durations.
- **A changed count is reviewed as a change of expected behaviour**, like every snapshot.

### Ceilings

- **The embedded Console stays under 1 MB brotli,** and its initial route under the ceiling area 12 sets. Only the owner moves a ceiling.
- **Resident memory stays under its ceiling per OS** for an Attached engine idle after `hello`, an idle Daemon, and a 1,000-page HTTP crawl of the local Proving ground. The check runs in the Release suite.
- **Each Product's executable has its own startup benchmarks and its own ceilings** for an idle Attached engine and an idle Daemon; the 1,000-page crawl runs on Foxhound's.
- **The Console's JS heap after 10,000 streamed events,** after a forced garbage collection, is within 10% of its value after 1,000.

### Where they run

| Check | Suite |
|---|---|
| Count snapshots in unit tests | Quick |
| Count snapshots against the binary, render and DOM counts, the Console leak check, the Python import set | Pre-merge (Linux x64); Release on every cell |
| `xtask perf` against the base, and benchmark admission | Pre-merge (Linux x64); Release adds Linux arm64 |
| Resident-memory ceilings | Release, every OS |
| Drift check against the last release | the daily test-health job |
| Performance series | a nightly Live check on the owner's machine when it is on, labelled with that machine; the rented reference host is declined while there is no revenue (ADR 0076) |

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
| | `sandbox_linux_unittests` on both builds (ADR 0074) | Chromium build gate | Linux |
| | renderer check on the signed bytes | release pipeline | every OS |
| Egress | SSRF and hostile-input cases in-process | Quick | every OS |
| | the same cases end to end | Pre-merge; Release | Linux; every OS, rebinding end to end on Linux only |
| Endpoints and the peer link | endpoint table; protocol fuzzing; MCP conformance over HTTP (placed by ADR 0070) | Pre-merge; Release | Linux; every OS (MCP conformance: as ADR 0070) |
| | ZAP baseline of the Console | Release | Linux |
| Secrets | redaction property tests | Quick | every OS |
| | Secret canary searches | the suite of the test that produces the output | its OSes |
| | `gitleaks` over job outputs | a step of every CI job | every OS |
| File outputs | path traversal cases | Pre-merge; Release | Linux; every OS, each OS's own cases |
| Audit log | tamper property tests | Quick | every OS |
| Reset gate | ADR 0001's planted Secret canaries | as ADR 0001 sets | as ADR 0001 sets |
| Unsafe code | ASan run of crates that opt into `unsafe` | Release | Linux glibc |
| Supply chain and workflows | ADR 0032's gates; image digest pins; CodeQL once public; `gitleaks` over commits until then | Pre-merge | Linux |

**Before the repository is public** (ADR 0032), the Release suite, with every part above, runs on manual dispatch before each milestone's owner test and before any release. CodeQL, GitHub secret scanning and push protection, and the nightly watch jobs start when the repository becomes public. Fuzzing runs within the plan's included minutes; paid minutes are declined while there is no revenue (ADR 0076).

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
- **An open fuzz crash blocks the promotion of the next minor** until it is fixed or triage closes it as not a vulnerability. Patches are not held. Never post a private result (the Private component run or fuzzing) on a public commit. The release pipeline reads them from the private repositories when each step starts, with the read-only private gates App: the Private component run for candidates and patches, and also open fuzz crashes for promotion to `x.y.0`. A promotion held while every public gate is green does reveal that a fuzz crash is open; this is accepted, since it shows neither the target nor the input (ADR 0067).
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

- **Keep SSRF cases in one table, as data, and run it at every layer:** the resolved-IP classification in-process, the HTTP tiers, the egress proxy, browser Sessions for every request type of ADR 0014, raw CDP, a Lent page whose client also tries a browser context with its own proxy and an extension (ADR 0089), the relays of ADRs 0042 and 0044, a Borrowed browser, Plugin traffic, and Sinks and webhooks under "no private endpoints".
- **Cover every family:** encodings; IPv6 forms (IPv4-mapped, NAT64, 6to4, Teredo, zone IDs); every IANA special-purpose block that is not globally reachable; names with mixed answers; redirects of every kind and to non-HTTP schemes; rebinding; the upstream proxy path.
- **Assert that no connection reached the target**, from the Proving ground's connection-recording fixture, and the typed refusal.
- **Test rebinding against the Proving ground's DNS fixture**: end to end on Linux in a network and mount namespace whose resolver points at it; in-process on every OS with a scripted sequence of answers. Never use a public rebinding service.
- **Keep the IANA special-purpose registries committed with their date.** A nightly watch job, which runs none of the product, opens an issue when they change.
- **Test the hostile-input set where each case can happen:** CR, LF or NUL in header values (Recipes, Sinks, webhooks, CDP); a domain-bound Secret across a cross-origin redirect; path traversal into every file Goodfetch writes from outside input, with each OS's cases; external `$ref` in extraction schemas; no process spawned to compare page content (checked under a seccomp filter that refuses `execve`); decompression bombs, endless bodies, deep nesting and oversized messages ending in their typed limit errors.

### Endpoints

- **Run the endpoint table against every listener:** the Console, the Daemon's socket and WebSocket, the HTTP listener, the ops listener, MCP over HTTP, the Live view, raw CDP, the Attached engine's private endpoint, an Exposed Daemon and the peer link.
- **Cover every row:** missing, wrong, expired, revoked, out-of-scope and other-listener tokens, and tokens in a query string; a foreign `Host`; a foreign or missing `Origin` on the request and the WebSocket handshake; plain HTTP where TLS is configured; `X-Forwarded-*` from an unconfigured address; each listener's own rules (ADRs 0031, 0049, 0050, 0055, and MCP's audience and no passthrough). Check each refusal in the Audit log.
- **Fuzz the protocol from the committed JSON Schema bundle** with `proptest` and `proptest-state-machine`, over every transport. Every invalid call gets a typed error and changes no state.
- **Run the MCP conformance suite against MCP over HTTP**, in the suites and on the platforms [ADR 0070](docs/adr/0070-mcp-adapters-and-integrations-are-tested-through-their-platforms-own-clients-against-a-real-engine.md) gives it.
- **Run a ZAP baseline scan of the Console in the Release suite:** image by digest, `-silent`, `allow_issue_writing: false`, FAIL rules committed, the crawl bounded by depth rather than time, under ADR 0043's deny so its Egress canary catches a call home.

### Secrets

- **Plant a Secret canary for every kind of secret** (typed password, TOTP seed, emailed or texted one-time code, proxy password, HTTP authentication password, a header or JSON value sent by `page.request` or a route, a value a `caller:` resolver returns, model API key, webhook signing secret, Sink credential, Session cookie, endpoint token, run data key), through the Proving ground's login flows and every Secret provider (ADR 0092).
- **Search every output for every Secret canary**, raw and JSON-escaped, URL-encoded, base64 and hex, and inside the Basic `Authorization` and `Proxy-Authorization` headers the engine builds. The outputs are:
  - logs, Run traces, HAR files and console captures, and screenshots and recordings where masking applies;
  - the Audit log, the Effect journal, and Checkpoints outside their encrypted fields;
  - Exports and Notifications;
  - JSON-RPC errors, error messages in run state, and model requests;
  - the Console's offline cache, `doctor`'s file, and the SDKs' `repr`, `toString` and errors;
  - what a call returns and an event carries: reads such as `inputValue`, page function results, extracted values, Records, response bodies, and page and network events, for every canary but a one-time code, masked only where it was typed, and a Session cookie, which `cookies()` and network events return by design;
  - Plans (ADR 0092).

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
| Independent security assessment | before 1.0 and each later major; declined while there is no revenue (ADR 0076), the manual threat-model review stands in | An outside tester attacks the boundaries; each finding gets a fix and a regression test | a report linked from the release notes |
| Confinement where CI cannot run | each minor's release candidate | WSL1's refusal; Bridge Sinks under Docker Desktop and Podman machine on macOS and Windows; the documented host-firewall recipes for `direct` | the release pull request |
| Passkeys on real authenticators | each minor's release candidate | Sign-in, step-up and recovery on a platform authenticator and a hardware key | the release pull request |

## Console and docs site

Decided in [ADR 0068](docs/adr/0068-the-console-is-tested-in-real-browsers-and-served-by-the-real-binary.md). Which accessibility, localization and visual checks run on these runners is in [Accessibility, localization and visual regression](#accessibility-localization-and-visual-regression).

### Where each test lives

Each suite contains the one before, so the table gives the first suite that runs each test; [Browsers](#browsers) says which browsers each suite adds.

| Path | What it tests | Runner | First suite |
|---|---|---|---|
| `console/src/**/*.test.ts` | logic with no DOM | Vitest `unit` (Node) | Quick |
| `console/src/**/*.test.tsx` | one component | Vitest `component` (Browser Mode) | Pre-merge |
| `console/src/**/*.stories.tsx` | each state of a component, rendered as a test | Vitest `component` (Storybook's addon) | Pre-merge |
| `console/tests/screens/` | the whole Console against the in-memory adapter of the protocol | Vitest `component` | Pre-merge |
| `console/tests/served/` | the Console served by the binary | Playwright Test | Pre-merge |
| `console/tests/journeys/J-NNN.spec.ts` | the `console` variants of Journeys ([Journeys](#journeys)) | Playwright Test | Pre-merge |
| `conformance/console/` | the Console conformance suite | Playwright Test | Pre-merge |
| the live-view packages' `*.test.ts(x)` | `<goodfetch-live-view>` and its React wrapper | Vitest Browser Mode | Pre-merge |
| `website/src/**/*.test.ts` | the docs site's build code | Vitest (Node) | Quick |
| `website/tests/` | the built docs site | Playwright Test | Pre-merge |
| `docs/**/*.md(x)` code blocks | the docs code samples, extracted by `xtask docs-samples` | `twoslash` in the docs build; the TypeScript SDK's Vitest, the Python SDK's pytest, rustdoc, trycmd, the MCP server's tests | Pre-merge |

### Components

- **A test that renders or needs the DOM is a `.test.tsx` file** and runs in Vitest Browser Mode with the Playwright provider. A `.test.ts` file runs in Node and never touches the DOM. Never jsdom or happy-dom.
  _Why:_ Base UI's positioning, focus and pointer handling need a real layout.
- **Mount with `vitest-browser-react`, find with `vitest/browser` locators, act with `vitest/browser`'s `userEvent`.** Never `@testing-library/user-event`.
  _Why:_ events are then real browser input sent by Playwright, and the locators are Playwright's.
- **Use the shared Vitest fixture of `@goodfetch/test-support`; it resets hover in `beforeEach`** (`userEvent.unhover(document.body)`).
  _Why:_ the Playwright provider keeps the pointer position and hover state between tests of one file, and tests run shuffled.
- **The engine is the in-memory adapter of the protocol from `@goodfetch/test-support`, passed to the Console's Client as its transport and body fetcher.** Never MSW, never a patched global `WebSocket`. A call the test did not answer fails the test, and every message is validated against the schema bundle.
- **Test data comes from builders with literal defaults.** Override only what the test asserts on; no random data.
- **Each state of a component has a story** (`*.stories.tsx`, Storybook 10 until Storybook 11 is stable and supports TypeScript 7). Stories hold no assertions and no play functions. Behaviour goes in `.test.tsx`, which may reuse a story through `composeStories`.
- **Storybook's docgen is `react-docgen`.** Never `react-docgen-typescript` or the experimental docgen server, which need the TypeScript compiler API.

### Browser state

- **Set colour scheme, reduced motion, locale, time zone and online status through Playwright:** context options (`colorScheme`, `reducedMotion`, `locale`, `timezoneId`), `page.emulateMedia` and `context.setOffline` in served tests; the provider's context options and the `@goodfetch/test-support` browser command in component tests.
- **Pass page visibility to the Console's Client as a value**, like its transport. Playwright cannot emulate it. Served tests of the idle stop close the page instead.
- **Never redefine a property of `document`, `navigator` or `window`.** oxlint's `no-restricted-properties` refuses `Object.defineProperty`, `Object.defineProperties` and `Reflect.defineProperty` in test files, and `vi.stubGlobal` is banned ([Writing a test](#writing-a-test)).

### The served Console

- **Browser tests of the Console as users get it run on Playwright Test against the executable built from the commit.** Never against a Vite server.
- **Each test starts its own Daemon** through the `@goodfetch/test-support` fixture (`goodfetch-test-env`), with its own home, port 0 and fresh browser context.
- **Test each Product's Console build served by its own executable:** a screen both Products carry on Collie's, Foxhound's screens on Foxhound's, and a route walk on Foxhound's that checks every route of Collie's build is there and renders.
- **Sign in the way a person does:** the binary's single-user sign-in link, Playwright's virtual authenticator for passkeys and step-up, a local OIDC provider pinned by digest. Never write a cookie or storage state yourself.
- **Make faults where they happen:** `routeWebSocket` with `connectToServer()` for a dropped or stalled connection, the real process for an engine fault. `routeWebSocket` replaces the page's `WebSocket` with a shim that forwards to the real binary; use it only to drop or delay real messages, never to invent one.
- **Find by role, label or text.** `data-testid` only where an element has no accessible role or name; never CSS classes or XPath.
- **Wait with web-first assertions and `expect.poll`.** Never `waitForTimeout` (oxlint refuses it) or `networkidle`.
- **A console error, an uncaught page error, a CSP violation or a failed same-origin request fails the test**, through the shared fixture.
- **`toMatchAriaSnapshot` and Vitest's ARIA snapshots follow [Snapshots](#snapshots).**
- **Tests run under the production CSP.** The binary has no switch that relaxes it, and Playwright's `bypassCSP` is never set: the shared fixture fails a test that sets it, and in test files oxlint refuses `newContext`, `browser.newPage`, `launchPersistentContext`, `launch`, `connect` and `connectOverCDP`, so every browser and context comes from the fixture. `context.newPage()` is allowed; API calls use the fixture's `request`.
- **Keep traces on failure** and upload them with the JUnit files.

### The Console conformance suite

- **It runs the Console against a target given as configuration:** each Product's Daemon, serving its own Console build, and the Stub gateway here, Goodfetch Cloud's gateway from the private repository. The Stub gateway serves Foxhound's Console build and starts each run's engine from the executable of the run's Product (ADR 0096). Its cases cover the rule that picks each run's Product: a Collie Client's run lands on Collie's engine and its `crawl.start` answers `not_in_product`; a Foxhound Client's run lands on Foxhound's; a Recipe that names Foxhound lands on Foxhound's whoever starts it; a run started from the Console lands by what its target names; Foxhound's screens show only for a run on Foxhound's engine, and a Foxhound-only call on a run of Collie's engine answers `not_in_product`, naming the run's Product; a Recipe that names Collie and calls `crawl.start` answers `not_in_product` on Collie's engine with the `fix` to name Foxhound in its manifest and no install command. Each target run writes its own result document with `kind = "conformance"`, like a Journey ([Journeys](#journeys)): its Playwright reporter derives each outcome, and the Rust reader recomputes it. It writes no case log.
- **Add a case when the Console starts to need something new from its target.**

### Browsers

- **One Playwright version for the whole pnpm workspace**, shared by Vitest's provider.
- **The Console's and the docs site's tests run on one Node line;** their matrix is browsers.
- **Pre-merge:** Chromium on Linux. **Release:** components, the live-view packages, served tests, Journeys and conformance on every OS runner with every browser Playwright ships for it, plus phone emulation on Chromium and WebKit for served tests; the docs site's build, lychee and Playwright on Linux, on Chromium, Firefox and WebKit; the docs code samples as [Docs code samples](#docs-code-samples) says.

### Docs site

- **Docusaurus throws on every broken reference:** `onBrokenLinks`, `onBrokenAnchors`, `onDuplicateRoutes` and `markdown.hooks`' `onBrokenMarkdownLinks`, `onBrokenMarkdownImages` and `onUnusedMarkdownDirectives`.
- **Every kept version is built, and every page has its `.md` twin, an `llms.txt` entry and its content in `llms-full.txt`.**
- **lychee runs with `--offline --include-fragments --root-dir <root>` in the suites**, over the built site copied to `<root>/docs/`, so absolute `/docs/` links resolve. External links are the Live check `docs-external-links`.
- **Playwright tests the built site served locally under `/docs/` on port 0,** never a Vercel preview: every `sitemap.xml` page of every kept version answers 200 with no console, hydration or same-site request error, and the interactive components work. Search is tested against an index built with the site; a hosted search service is tested only by `docs-deploy-smoke`.
- **After each production deploy, the Live check `docs-deploy-smoke`** checks the deployed pages, a `.md` page, `llms.txt` and `llms-full.txt` with their content types.
- **`typos` checks the docs' spelling.** No Vale, no cspell.

### Docs code samples

- **Give every fenced block in `docs/` a language;** use `text` for output and prose. The docs build fails on an empty info string.
- **`xtask docs-samples` extracts every sample** from `.md` and `.mdx` pages into `target/docs-samples/`, with a manifest. The docs build's remark plugin lists every sample block it sees; a difference between that list and the manifest, or a runner whose case count differs from the manifest, fails the check.
- **Tag a block that can't run `no_run`** after its language. A `rust`, `ts` or `python` block is still compiled, type-checked or linted; a `console` block or an MCP pair is only listed in the manifest.
- **Mark MCP samples `json mcp-request` and `json mcp-answer`.** Other `json` blocks are not samples.
- **Each surface's tests run its samples:** `twoslash` type-checks `ts` and `tsx` in the docs build (our own remark plugin, on the TypeScript 6 alias) and the TypeScript SDK's tests run them; Ruff lints and the Python SDK's pytest runs `python`; `#[cfg(doctest)]` includes in `goodfetch-collie` and `goodfetch-foxhound` run `rust`; trycmd runs the `.trycmd` files from each executable crate's `tests/it` (`...` for any lines, `[..]` within a line); an MCP server test plays each `mcp-request` and `mcp-answer` pair through `rmcp`.
- **Run samples against the binary built from the commit and a local Proving ground,** never the online one. Hide setup with the tool's own convention (`# ` in Rust, `---cut---` in Twoslash).
- **Run a sample against the executable of the Product its page documents,** Collie's for what both Products carry.
- **Only `docs/` (`next`) runs.** Frozen versions were tested at their release.
- **Pre-merge:** the affected samples on one runtime. **Release:** every sample on its surface's own matrix ([SDK and CLI tests](#sdk-and-cli-tests), [MCP, framework adapters and Integrations](#mcp-framework-adapters-and-integrations)).

## Feature coverage

Decided in [ADR 0072](docs/adr/0072-every-feature-in-a-generated-inventory-has-a-passing-test-and-code-coverage-and-mutants-are-signals.md). Coverage means every Feature has a passing test, never a percentage. Suites are those of [Suites, commands and time budgets](#suites-commands-and-time-budgets); quarantine is that of [Flaky tests and Live checks](#flaky-tests-and-live-checks).

### The Feature inventory

- **Never edit `features/inventory.json` by hand.** `xtask codegen` generates it from ADR 0037's matrix, `features/stories/`, `features/rows.toml`, `features/areas.toml` and the contracts the code defines, and `xtask codegen --check` fails on drift.
- **Every generator reads source files or Rust types in process.** `screen:` comes from the Console's route definitions read statically; if they can't be, from a route list committed in the Console's package, which the Console's Pre-merge build checks for drift. The Quick suite never builds the Console to get it.
- **Write a Feature as `<kind>:<id>`.** Declared kinds: `row:` (an ADR 0037 matrix row that is not `excluded`) and `story:<area>-<n>` (a spec's user story). Generated kinds: `operation:`, `error:`, `event:`, `setting:`, `command:`, `exit:`, `format:`, `plugin-kind:`, `block-cause:`, `sink:`, `mcp-profile:`, `skill:`, `adapter:`, `integration:` and `screen:`.
- **A contract you add is a Feature in the same pull request,** and that pull request adds a test that declares it. The Quick suite fails until it does.
- **Never renumber or reuse a story ID.** A new story takes its spec's next number; a removed one stays with `status = "retired"` and a `reason`.
- **Give every declared Feature a milestone** (`/to-tickets` sets it), and every generated Feature an owning area through `features/areas.toml`.

### Declaring what a test proves

- **Declare a Feature only when the test asserts its outcome**, never one the test merely passes through.
- **Declare it in the runner's own idiom**, never by a file or name pattern:
  - Rust: `/// Covers: operation:runs.start, error:run_not_found` on the test function, or `//! Covers: …` on a test module for every test in it;
  - Vitest: the Feature ID as a tag, declared from the inventory, with `strictTags` on;
  - pytest: `@pytest.mark.covers("<feature id>")` or a module's `pytestmark`, under `strict_markers`;
  - Playwright Test: an annotation `{ type: "covers", description: "<feature id>" }` per Feature;
  - Journeys, conformance cases and Client conformance cases: their `covers` list.
- **A test that drives the protocol and declares an `operation:`, `error:`, `event:`, `command:` or `exit:` Feature must reach it.** The exercise audit reads `rpc`'s dispatch records (the engine's structured log for the binary, the test `tracing` subscriber for in-process tests over the in-memory transport) and test support's command record, and fails a declaring test that never called the operation, received the Error code or event, ran the command or saw the exit code.
- **A unit test that never reaches `rpc` may declare the `error:` or `event:` Feature it asserts,** but must assert the Error code or the event itself. The audit can't see it; the review checks it.

### What counts as tested

- **Every Feature is tested, excepted or untested.** The gates fail only on untested Features.
- **A Feature is tested** when, on `main`'s latest Release suite run, at least one test declares it, every declaring test ran and passed in every configuration it runs in, none is quarantined, every declaring Journey's variants of reached milestones are `proved`, and every declaring conformance case's capability is `proved`.
- **A story with `journey = true` also needs a proved variant of a Journey that covers it,** and every story needs a declaring test above the unit level ([Which stories need a Journey](#which-stories-need-a-journey)).
- **Test a Feature once.** The per-surface completeness and conformance of [SDK and CLI tests](#sdk-and-cli-tests) and of the surface ADRs, and a Journey's variants, prove each surface.
- **Once, not once per Product.** The inventory records the Products that carry each Feature, from ADR 0087's generated table of Foxhound's names, and `xtask features report` gives each Product's view.
- **A quarantined test leaves every Feature it declares untested** until its fix merges.

### Exceptions

- **List a Feature no pinned test can prove in `features/exceptions.toml`;** it is then excepted, never tested. Give it its `evidence` (`manual`, `live`, `check` or `none`), a `reason` and a `link` to the owner-test step, Live check, CI job or place where it is tested.
- **Never except a Feature a pinned test can reach.** The release pull request lists every exception, and the owner test lists the `manual` ones as manual steps.

### The checks

| Check | Suite or gate | Fails when |
|---|---|---|
| Feature check | Quick | the inventory is stale, a declaration names an unknown or retired Feature, a generated Feature has neither a declaring test nor an exception, a non-excluded matrix row has no milestone, a generated Feature has no area or two, or an exception is incomplete or names a Feature a test also declares |
| Exercise audit | Pre-merge (tests that ran), Release (all tests) | a declaring test never reached its Feature |
| Milestone gate | the owner test (ADR 0058) | a generated Feature, or a declared Feature due at the milestone, is untested (neither tested nor excepted) |
| Release gate | `xtask release` | a Feature present in the release is untested (neither tested nor excepted) |

- **Run `just features`** to see what the changed packages declare and miss.

### Code coverage and mutation testing

- **Never gate on a percentage:** no `--fail-under-*`, no coverage or mutation score in any check, badge or pull request, and no coverage service.
- **The coverage job runs in no suite** on Linux x64, weekly (nightly once the nightly tier is on): cargo-llvm-cov over nextest and over every run of the instrumented binary through `show-env`, Vitest's `v8` provider on Node and in Browser Mode, coverage.py with branches. While the repository is private, it has a 60-minute timeout and runs only while the month's count toward the ceiling of [Flaky tests and Live checks](#flaky-tests-and-live-checks) is below 550 minutes. Its results never decide a Feature's status. Run `just coverage` locally.
- **Read coverage for gaps.** A function no test executes that hides a behaviour with no Feature gets a story, a Feature or a test.
- **Every pull request that changes Rust code runs cargo-mutants `--in-diff`** with nextest on Linux x64, sharded within the 15-minute job budget, in a job that never fails on a survivor and is not part of "all green". While the repository is private, it runs at most four shards per pull-request run and stops at the same 550 minutes; the Verification section then says it was skipped.
- **List each surviving mutant of your changed code in the Verification section,** with the test that now kills it or why it stays (an equivalent mutant; caught only by a doctest, by a Journey in another runner, or by another OS's test).
- **Full mutation runs are weekly once the nightly tier is on:** cargo-mutants over the workspace, StrykerJS on the TypeScript SDK runtime and the Console's logic (its `command` runner until a release fixes [stryker-js#6210](https://github.com/stryker-mutator/stryker-js/issues/6210), then its Vitest runner and an in-diff pull-request job), and mutmut on the Python SDK runtime on Linux.

## Accessibility, localization and visual regression

Decided in [ADR 0073](docs/adr/0073-accessibility-meets-wcag-2-2-aa-the-console-ships-ready-to-translate-and-screenshots-compare-in-one-pinned-image.md). The runners are those of [Console and docs site](#console-and-docs-site). Two product decisions in it await the owner's confirmation: V1 ships in English, ready to translate, and the docs publish an accessibility statement.

### Accessibility

- **The target is WCAG 2.2 AA** for the Console, the docs site and the Live view packages' own controls. The remote page inside the Live view is third-party content; the Live view's controls, name and announcements are not.
- **Every oxlint `jsx-a11y` rule is an error.**
- **Every Storybook story runs Storybook's a11y addon with `parameters.a11y.test = 'error'`** and the tags `wcag2a`, `wcag2aa`, `wcag21a`, `wcag21aa`, `wcag22aa` and `best-practice`, in light and dark.
  _Why:_ axe's WCAG 2.2 rules are off by default.
- **Use the shared Playwright fixture: it runs axe with the same tags at the end of every served test and `console` Journey variant.** Call `expectAccessible(page)` at each state worth checking on the way (an open dialog, drawer or menu). The docs pass runs axe on every `sitemap.xml` page of `next`, light and dark.
- **Never disable an axe rule in a test.** Except a false positive only in the shared exception file, with the rule, a selector, the reason and an open issue; the fixture fails when the issue closes or the selector matches nothing. "Incomplete" results go to the report and never fail.
- **Each screen has a keyboard walk test,** one named served case per `screen:` Feature of the inventory: Tab and Shift+Tab reach every element with an interactive role, focus never falls to `body`, Escape closes each overlay and returns focus to its trigger, and the ordered accessible names are a snapshot. At each stop, the element must look different focused and unfocused, and must not be entirely covered.
- **Check reflow, text spacing and motion with the shared helpers:**
  - at 320 × 256 CSS pixels there is no horizontal page scroll, except inside data tables and code blocks with their own scroll container;
  - with WCAG's text-spacing values and at 200% text, in the `component` project's screens, no text is clipped without a way to read it in full;
    _Why:_ served tests run under the production CSP, which refuses an injected style sheet, and Playwright has no option for either;
  - under `reducedMotion: 'reduce'`, `document.getAnimations()` holds no running animation after an action.
- **Every drag has a single-pointer alternative,** and its test performs the action through it.
- **Test announcements with `@guidepup/virtual-screen-reader`** in the `component` project: live regions, route titles and Live view control changes. Assert names and roles in the spoken log.
- **The Live view's escape key always returns focus to the Console** while a person holds control; a served test proves it and that every other key reaches the remote page.
- **The CLI's colour never carries information alone:** in a pseudo-terminal, the coloured output with ANSI sequences stripped equals the `NO_COLOR=1` output. `NO_COLOR` set to any non-empty value disables colour, an empty one does not, and `--color always` overrides it. The shared CLI helper fails any `tests/it` case whose piped output holds an escape byte (0x1B).
- **Accessibility tests declare no Feature** ([Feature coverage](#feature-coverage)): axe, keyboard walks, announcements and screenshots check how a screen is built, not its outcome. A screen's `screen:` Feature is declared by the tests of its behaviour. The keyboard walk has one served case per `screen:` Feature of the inventory, ending with the fixture's axe pass, so a new screen gets both.
- **Real screen readers run as the Live check `screen-reader`** (Guidepup, NVDA on Windows and VoiceOver on macOS), nightly, owned by area 12, blocking nothing directly; before each release candidate while the nightly tier is off.

### Localization

- **V1 ships in English.** The Console is internationalized: every interface string comes from its Lingui catalog with ICU messages.
- **Never hard-code a Console string.** `lingui/no-unlocalized-strings` runs through oxlint's JS plugins, and `lingui extract` in the Quick suite fails on an uncommitted catalog change.
- **Render data inside `translate="no"`:** URLs, names, IDs, page text and every value from the engine.
  _Why:_ browser translation then leaves data alone, and the pseudo-locale test tells data from interface text.
- **Run the `component` project and the served tests under `en-XA` and `ar-XB`** in the Release suite. Under `en-XA`, every visible text node is bracketed or inside `translate="no"`, no text is clipped, and the keyboard walk passes. Under `ar-XB`, the root has `dir="rtl"` and arrow keys in menus and tabs follow the direction.
- **Format only through the Console's formatting module,** which uses `Intl` with the active locale and time zone. Assert formatted strings only in the `unit` project. Browser tests assert the machine form (`<time datetime>`, `data-value`) or a role, never formatted text.
  _Why:_ each browser ships its own ICU and CLDR; the `unit` project runs on one pinned Node line.
- **Take locale and time zone cases from the shared table in test support** (`en-US`, `de-DE`, `fr-FR`, `tr-TR`, `ar-EG`, `hi-IN`, `ja-JP`; `UTC`, `America/St_Johns`, `Asia/Kathmandu`, `Pacific/Chatham`, `Australia/Lord_Howe`, and the DST gap and overlap days of `Europe/Paris` and `America/New_York`). Period Limits and Schedules use it for their zone cases.
- **The elsewhere project** reruns served tests and `console` Journey variants with `locale: 'tr-TR'` and `timezoneId: 'Asia/Kathmandu'` (Release).
- **A translation project** adds a page script that wraps every new text node in `<font>`, as page translators do; served tests must pass with no uncaught error (Release).
- **The CLI's human output reads no locale:** the snapshot commands give the same bytes under `LANG` and `LC_ALL` set to `tr_TR.UTF-8`, two `TZ` values print the same instant, and `--json` is UTC.
- **Engine results never depend on the host's locale or time zone** unless a setting names a zone. The Release suite reruns the engine's integration tests and the Journeys on Linux and macOS with `TZ=Pacific/Chatham` and `LANG=tr_TR.UTF-8`.
- **The shared Unicode record** (bidi text with marks, NFC and NFD forms, a ZWJ emoji sequence, an astral character, a zero-width joiner, a BOM, a tab and a newline in a field) round-trips byte for byte through every Sink conformance case and every Export format, and renders in a Console story.
- **The Proving ground's encodings and language families** cover the encodings and scripts ADR 0073 lists; their oracle holds the expected text in UTF-8.
- **Keyboard layouts and IME:** a served Chromium test drives the Live view with AZERTY, QWERTZ and dead-key pairs and a CDP `Input.imeSetComposition`, and compares a Proving ground page's record of events with literal expected events. Enter during a composition never submits a Console form or search.

### Visual regression

- **Compare a Reference screenshot only inside the pinned Playwright image, on Linux x86-64.** Visual tests are the `visual` projects of Vitest and Playwright, and nothing else schedules them.
  _Why:_ rendering differs by OS, architecture and font stack.
- **The shared Playwright config sets `updateSnapshots: 'none'` in the suites,** so no suite writes a Reference screenshot, even a missing one.
- **Never set a tolerance in a test.** Zero mismatched pixels at the default per-pixel threshold; a config check fails if `maxDiffPixels`, `maxDiffPixelRatio`, `allowedMismatchedPixels`, `allowedMismatchedPixelRatio` or `threshold` is set outside the shared config.
- **Make the screen deterministic:** take screenshots through the shared helper, which awaits `document.fonts.ready`; fix the instant (`page.clock.setFixedTime`, or Vitest fake timers), `locale: 'en-US'`, `timezoneId: 'UTC'`, the viewport, `deviceScaleFactor: 1` and `reducedMotion: 'reduce'`; use builder data with literal defaults. Mask only the Live view's frames.
- **Make hover, focus and pressed states with real `userEvent` input** in `.test.tsx` files, never a pseudo-states addon.
- **A Reference screenshot is neither a fixture nor a snapshot** in the sense of [Fixtures and data files](#fixtures-and-data-files) and [Snapshots](#snapshots): their size limit, their `.source.toml` and "CI never writes a snapshot" do not apply; its own rules below do.
- **Keep Reference screenshots beside their tests,** named without the platform, each at most 512 KiB (Quick). Clip a larger one to the region the test is about. No Git LFS. A run of every visual project fails on a Reference screenshot no test read.
- **Update Reference screenshots only through the `update-screenshots` workflow** on the pull request's branch, then `just screenshots-fetch <run>`, and commit them with your own sign-off, naming the run in the commit message. The workflow is the only writer and is not a suite job, so no suite writes one. `just screenshots-fetch` refuses a run whose image digest is not the pinned one or whose commit is not the branch's head. Never commit a Reference screenshot rendered outside the pinned image.
- **Locally, `just` runs the visual projects in the same image through Docker or Podman;** with neither, the step fails with an error naming the fix.
- **What is screenshotted:** every Storybook story in light and dark; the Console's `tests/screens/` under greyscale, text spacing and 200% text in Vitest's `visual` project in Release, where a style sheet can be injected; the served key screens ADR 0073 lists in light and dark at 1280 × 800, plus phone, Firefox, WebKit, forced colours, more contrast and both pseudo-locales in Release; the docs' page templates, light and dark, desktop and phone.
- **If the first fixtures ticket finds that a browser's screenshots do not repeat in the pinned image** (ADR 0064's 20 runs), that browser stays out of the `visual` projects, its served tests and axe keep running, and an issue owned by area 12 tracks it; the browser joins the `visual` projects once 20 runs repeat. Never add a tolerance instead. After that, a screenshot test that stops repeating is a Flaky test like any other ([Flaky tests and Live checks](#flaky-tests-and-live-checks)): quarantined, never removed.
- **No hosted visual service:** declined while there is no revenue (ADR 0076); screenshots compare in the pinned image.

### Where they run

| Check | Suite |
|---|---|
| `jsx-a11y`, `no-unlocalized-strings`, the `lingui extract` check, the formatting table, reference sizes, the tolerance check | Quick |
| axe on stories and served tests, keyboard walks, focus, reflow, target size, reduced motion, virtual screen reader, layout and IME tests, the CLI's colour and locale cases, docs axe, visual projects on Chromium | Pre-merge (Linux) |
| axe in served Console tests in every browser on every OS runner, text spacing and 200% text, pseudo-locales, the elsewhere and translation projects, the host-elsewhere engine run, the other screenshots, the orphan check | Release |
| `screen-reader` | Live check, nightly |

### Manual tests

Only these stay manual, on each minor's release candidate once the Console ships, recorded on the release pull request:

| Manual test | What a person does |
|---|---|
| Screen-reader pass | NVDA with Firefox and Chrome on Windows, VoiceOver with Safari on macOS and iOS, TalkBack with Chrome on Android, over the minor's Console Journeys and changed screens, some docs pages, and the CLI's common commands |
| Keyboard, zoom and contrast pass | Keyboard only, 400% zoom, the OS magnifier, a Windows contrast theme and speech input on changed screens, in Chrome, Firefox and Safari |
| Real IMEs and layouts | Japanese, Chinese Pinyin and Korean IMEs, AZERTY and a dead-key layout, in the Live view and Console fields, on macOS and Windows |
| Browser translation | Chrome's and Safari's page translation over the main screens |
| WCAG review of new screens | Names and instructions, error suggestions, consistent help, redundant entry, sensory-only cues; the accessibility statement kept current |

## Which stories need a Journey

Decided in [ADR 0075](docs/adr/0075-a-user-story-needs-a-journey-only-when-it-is-a-persons-goal-a-variant-can-reach.md). Journeys sit at the top of the pyramid, so a story gets one only when it is a person's whole goal that a Journey variant can reach. Lower tests prove the rest.

### The flag

- **Apply three tests to each story, in order, from its own "I want" and "so that".** The first test it fails sets `journey = false` and names that test in `not_journey`. A story that passes all three gets `journey = true` and no `not_journey`.
  1. **`actor`:** the actor is a person using a shipped surface (a user, a developer writing code against an SDK or the CLI, an operator, a Recipe author or user, a Plugin author, a Console user, the person behind an agent). Contributors, maintainers, the owner as releaser, areas, test support, Clients and Client authors, coding agents, Goodfetch Cloud, site owners and readers of the benchmark fail it.
  2. **`part`:** the story states a whole goal, a task the person sets out to do or see. It fails when its "I want" is one setting's effect, one Error code or a refusal of a wrong input, version, environment or configuration, one event, field or operation detail, or how the engine works. A stop at a Limit the person set, and the failure a person must see during a goal, pass it.
  3. **`reach`:** the outcome shows through a surface of [Journeys](#journeys), in an isolated variant on one host, with the Proving ground and the services test support gives every variant, and only a person's means (start, stop or kill a process, change a setting, run a command, add or remove a Daemon). It fails when the outcome needs an injected fault, an old released binary, an external account or service, Kubernetes, state outside the variant's Goodfetch home and processes, or the release pipeline's artifacts or channels. An older-format file from the Compatibility corpus is allowed.
- **Write the result in `features/stories/<area>.toml`:** `journey`, and `not_journey = "actor"`, `"part"` or `"reach"` when it is `false`. The Feature check fails on a missing, unknown or extra reason. The review checks each reason against the story's words.

### Evidence

- **Give every story at least one declaring test above the unit level**: an integration test (the Simulation, and multi-process and crash tests of the binary, included), a conformance case, a Client conformance case or a Journey. In-process crash-VFS, model-based and loom tests are unit tests. An exception also meets this rule. The level is read from where the test sits: [Levels and layout](#levels-and-layout) for unit and integration tests, `conformance/` for conformance cases, a `journeys` folder for Journeys. Otherwise the story is untested, with the cause "no test above the unit level".
- **Prove an unflagged story where its condition can be made:**
  - `actor`: the tests its contract needs (conformance or Client conformance cases, area 1's `xtask` tests, release rehearsal and smoke runs, `check` exceptions, the Proving ground's own tests);
  - `part`: an integration test of the package that owns it, or a test of the binary, asserting the setting, Error code, event or mechanism;
  - `reach`: the Simulation, crash and partition tests, multi-process tests, skew tests, release rehearsal and deployment smoke runs, or a `live` or `manual` exception.
- **Test a flagged story's edge cases, Error codes and limits lower down too.** Its Journey proves the goal only.

### Journeys for flagged stories

- **A Journey covers a story only when one of its `[[expect]]` checks asserts the story's outcome.**
- **When a flagged story's goal is an existing Journey's goal, or is its starting step (installing what the variant drives), add the story to that Journey's `covers`** with a check, and a step if the goal needs one. Keep the ID. For any other goal, add a Journey. There is no quota of Journeys per area.
- **A flagged story is due with its Journey at the later of its own milestone and the earliest milestone of a covering variant.** Until then its other declaring tests decide it. From then on it needs one proved variant.

## Goodfetch Chromium

Decided in [ADR 0074](docs/adr/0074-the-goodfetch-chromium-is-gated-where-it-is-built-against-a-control-build-of-the-same-revision.md). Suites are those of [Suites, commands and time budgets](#suites-commands-and-time-budgets); Live checks, diagnostic runs and the results store are those of [Flaky tests and Live checks](#flaky-tests-and-live-checks).

### The Chromium build gate

- **The build's inputs** are the files under `chromium/` that make the build (the Chromium version, ungoogled-chromium's tag, the fingerprint patch series, Goodfetch's own patches, the build scripts and arguments). **Its test inputs** are the WPT and Khronos pins, the expectations, the launch-arguments file and the gate's own workflow file, so a change to how the gate runs never passes on an old verdict. The digest manifest and the protocol JSON are outputs, in neither set. A pull request that changes either set is a Chromium pull request.
- **The Chromium build gate is a required check on every head:** every pull request (to `main` or a release branch, cherry-picks included) and the merge queue, never behind a path filter.
  - It passes fast when every digest in the committed manifest has a verdict attestation naming this head's two input tree hashes.
  - Otherwise, on any pull request, it builds and tests on every platform the build ships; when only test inputs differ, it fetches both builds and their packages by digest and reruns the tests.
  - In the merge queue, a mismatch fails; run the gate in the pull request.
  - Until the owner approves the first measured build and its manifest is committed, it passes and builds nothing.
- **Build where the gate runs, and nowhere else.** The gate builds from the lite and testdata tarballs of the version, checked against Google's hashes. A later job attests and publishes the build, its chromedriver, the Control build and both builds' test packages; only the Goodfetch Chromium and chromedriver are ever shipped. Split a file over GitHub's 2 GiB release-asset limit into parts, each with its digest.
- **Commit the manifest of each platform's digests and the protocol JSON the gate produced.** Each build has a build attestation (build-input tree hash, source commit, workflow), and every passing gate run, a build or a rerun of the tests alone, adds a verdict attestation on each digest (both tree hashes, source commit, workflow). The gate refuses a digest whose build attestation names other build inputs. A fork's Chromium pull request is rebuilt from a branch of the main repository.
- **Never rebuild in a suite or a release.** Suites fetch the build by digest; the release pipeline signs the published bytes.

### The Control build

- **The Control build is the gate's tree with Goodfetch's patch series reverted** (fingerprint-chromium's patches and Goodfetch's own), rebuilt incrementally after the published build, with the same build arguments. ungoogled-chromium's own patches stay.
- **Run every gate test on both builds,** each against its committed expectations, with the same launch arguments.
  _Why:_ a failure on the Goodfetch Chromium alone is the patches'; on both, it is upstream's. Running the Control build is never a retry: the Goodfetch Chromium's result stands.

### What the gate runs

- **Chromium's own suites:** those `gn refs --all --testonly=true --type=executable` finds linking a patched file, within `unit_tests`, `components_unittests`, `content_unittests`, `blink_unittests`, `blink_platform_unittests`, `headless_browsertests`, `content_browsertests` and `browser_tests`; plus `sandbox_linux_unittests` on Linux, always. Unit suites and `headless_browsertests` run whole; `content_browsertests` and `browser_tests` run the test files in the directories the series touches, listed in the gate's report.
- **Pass `--test-launcher-retry-limit=0` on every gtest run.** The launcher retries once by default.
- **Shard with the launcher's own hash sharding,** from packages made from `gn`'s runtime deps, on standard runners; a package too big for one runs on the build runner.
- **web-platform-tests:** `./wpt run chrome` at the pinned WPT commit (the commit of wpt.fyi's first Chrome Stable run on the first day the adopted major's version ran there; security updates keep it), with the chromedriver built from the same tree, on the directories `chromium/surfaces.toml` names, in `--chunk-type hash` chunks. wpt.fyi's results never gate.
- **Launch the browser as the engine does:** the `engine` Fingerprint method's arguments with a fixed Identity profile seed, from the file `xtask codegen` writes from the engine's code.
- **WebGL:** Khronos' `webgl1_conformance` and `webgl2_conformance` through Chromium's `run_gpu_integration_test.py` with `--browser=exact` and typ's `--retry-limit=0`, on SwiftShader with `--enable-unsafe-swiftshader`.
- **Size:** each platform's compressed archive. Growth of more than 5 MB over the previous build needs ADR 0057's "Size increase" section in the pull request body, accepted by the fresh review on the exact head.

### Expected results

- **Keep one current set, in each runner's own format,** under `chromium/expectations/`: filter files per suite and platform for gtests (one `-Suite.Test` line per entry, with its comment), and two wptrunner metadata trees, one per build, written by `wpt update-expectations` and reviewed. Git versions them with the pin.
- **Give every entry one category and its evidence:**
  - `upstream`: fails on the Control build too, with a link;
  - `upstream-flaky`: varies on the Control build;
  - `intended`: a deliberate effect of a named patch, on the Goodfetch Chromium only, naming its surface row.
- **The two WPT trees differ only in `intended` entries.** A check enforces it.
- **An `upstream` entry whose test passes on the Control build fails the gate,** and so does an expected failure that passes. Each Chromium pull request runs the excluded `upstream` gtests once on the Control build and fails on any that pass.
- **`upstream-flaky` entries are upstream exclusions, not the quarantine list:** they exclude Chromium's tests, never Goodfetch's.
  - Generated entries come from LUCI Analysis (a flake rate of 1% or more over 30 days), regenerated in each Chromium pull request that changes the Chromium version and committed with the command that made them, one tracking issue per file.
  - Every other entry needs a diagnostic run showing both outcomes on the Control build, and has its own issue, an upstream link and a date.
  - Each Chromium pull request that changes the Chromium version rechecks those entries with 20 runs on the Control build of each platform they name, inside the 90-minute test stage. 20 passes on a platform fail the gate until the entry is removed there; an entry not renewed by a recheck showing both outcomes expires after 8 weeks.
  - They block no milestone or release; their issues sit with area 8.
- **Never enter flakiness of the Goodfetch Chromium alone.** It is a patch bug: fix the patch. An entry excludes a flaky test; it never widens a Goodfetch Chromium expectation to "pass or fail".
- **List the entries added and removed** in the Chromium pull request's description.

### Goodfetch's own tests of the build

- **Give every patch file a row in `chromium/surfaces.toml`,** with the surface it changes, its WPT directories and its Goodfetch tests; infrastructure patches (build fixes, background traffic, the variations seed) get rows of their own. The Quick suite fails on a patch without a row or a row without a test.
- **Check Identity consistency on the Proving ground:** each surface reports the Identity profile's value in the main frame, a cross-origin iframe, dedicated, shared and service workers, and the HTTP headers; the same seed gives the same values across launches, different seeds give different ones, and nothing changes within a Session.
- **Check version truth:** user agent, client hints' full version list and `navigator.userAgentData` report the pinned version, and the build's captured HTTP profile matches its own ClientHello and HTTP/2 settings.
- **Check codecs and fonts:** every codec claimed decodes a Proving ground sample; every font the profile lists renders differently from the fallback, and no host font outside the profile is reachable.
- **Check that the browser is quiet:** under a NetLog, a launch, an idle minute and a Proving ground navigation connect only to the Proving ground, and a request to a Google host mapped by `resolve` carries the shipped variations seed's `X-Client-Data` header.
- **Check the CDP protocol in the Quick suite:** every method, event and parameter the driver uses is in the committed protocol JSON.
- **Fail on any crash dump:** each test that launches the build gives it a crash-dump folder in the test's Goodfetch home.
- **Smoke the signed bytes** in the release pipeline, on every OS, before publishing: launch, the CDP handshake, the renderer sandbox check and one stealth fixture.
  _Why:_ signing changes the bytes, and a wrong macOS entitlement breaks a renderer while every signature check passes.

### Where each part runs

| Part | Where |
|---|---|
| `chromium/surfaces.toml` check; CDP protocol check; the no-retry check's Chromium launcher, wptrunner and typ flags | Quick suite |
| The patch series applies to the pinned version (`validate_patches.py`, fetching only the touched files) | Pre-merge suite on Linux, when a build input changes |
| Identity consistency, version truth, codecs and fonts, the quiet browser, the renderer checks, the guard matrix and the stealth fixtures, on the build by digest | Pre-merge suite on Linux when affected; Release suite on every OS |
| Build, Chromium's suites, WPT and WebGL on both builds, expectation checks, size | Chromium build gate: required on every head, building and testing only on a Chromium pull request, every platform |
| Signed-bundle smoke | release pipeline, every OS |
| Chromium patch preview (weekly and at each new Beta major); Chromium build currency (daily) | Live checks |

- **A new major or security update ships only when** the gate is green on every platform, the Chromium pull request passes the Release suite, and the patch passes ADR 0032's gates and the signed-bundle smoke. The online conformance still never blocks.
- **Triage a red gate without a retry:** read the report's result on both builds, let the gate's diagnostic run (20 runs of each failing test on each build, sharded, 60 minutes per job) tell flaky from broken, then fix the patch or add the entry the evidence supports. Never skip a test without an entry.
- **A held major never weakens the gate.** The pinned build keeps shipping. If a patch still cannot pass when the next Stable version ships, the owner decides whether that major ships without it.
- **Budgets:** build ≤ 4 h p50 per platform (6 h cap; stage a build that cannot fit over chained jobs), test stage ≤ 90 min p50 per platform (3 h cap per job). The gate's hosted runner minutes, the macOS runner and a full gate on Beta majors are declined while there is no revenue (ADR 0076); the gate's free design is decided before M3.
- **Results** go to the results store under the tier `chromium-build`, as JUnit XML.

## Free tiers and local secrets

Decided in [ADR 0076](docs/adr/0076-tests-and-ci-run-on-free-tiers-and-ai-calls-go-through-the-vercel-ai-gateway.md).

- **Spend nothing until the owner declares revenue.** No job, build, account or service that bills is created or switched on. Where a paid option would be better, choose the free one and record the paid one as declined.
- **Run CI on GitHub's free standard runners** within the private plan's included minutes, and on a self-hosted runner on the owner's machine where a build needs more. A self-hosted runner never runs a pull request from a fork.
- **Time on the owner's machine, never publish it as the reference class.** The Performance series and the release-candidate benchmark run on the owner's machine (Apple M3 Max) when it is on, labelled with that machine; the rented reference host and the benchmark's paid services (proxies, CAPTCHA solving, model spend) are declined, and published reference-class runs wait for revenue.
- **Use free tiers for outside accounts.** Real SaaS Sink accounts and Integration platform accounts use free plans only; a Sink with no free tier keeps its emulator suite and its Live check waits. LocalStack runs on its free open-source licence; if that is refused, the affected Sinks use free emulators such as MinIO, or wait. Translation uses pseudo-locales only.
- **Keep local secrets in `.env`, written by `scripts/env-setup.sh`.** It is git-ignored and mode 600; `.env.example` lists every name, what it is for, the milestone that needs it and the page that creates it. Add a new secret to `.env.example` in the pull request that first needs it.
- **Never read `.env` from a test.** Test support clears the environment (ADR 0059); only the recipes that run Live checks load `.env`, each on its own, so the `justfile` never uses a global `set dotenv-load`. CI reads the same names from GitHub Actions secrets.
- **Never print, log or commit a secret's value,** including in a failure message or a recording; Secret canaries and `gitleaks` check it (ADR 0067).
