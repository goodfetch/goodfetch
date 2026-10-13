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
9. [Test-first process](#test-first-process)
10. [Review checklist](#review-checklist)
11. [Suites, commands and time budgets](#suites-commands-and-time-budgets)
12. [Upgrades and version mismatches](#upgrades-and-version-mismatches)

Sections 1 to 7, 9 and 10 are decided in [ADR 0062](docs/adr/0062-tests-sit-beside-their-code-and-share-one-test-support-written-once.md).

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
- **A flaky test is quarantined with an issue, never retried silently.**

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
- **An old Client runs its own release's journeys** from its tag. A journey that asserts something outside semver is fixed on `main` and excluded by ID, with a reason, in `compat/skew-exclusions.toml`.
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
