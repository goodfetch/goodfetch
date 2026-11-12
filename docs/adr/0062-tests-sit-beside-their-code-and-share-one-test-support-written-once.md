# Tests sit beside their code, cross one seam each, and share one test support written once

Every package keeps two kinds of test in two fixed places: **unit tests** beside the module they test, through that module's interface, and **integration tests** in the package's own `tests/` folder, through the package's public interface. End-to-end tests follow the Journey catalog. Shared setup is written once, in Rust, in an internal `goodfetch-test-support` crate that depends on no Goodfetch crate. TypeScript and Python reach that setup through a `goodfetch-test-env` executable, from two thin packages under a new `test-support/` folder. Agents write tests first, at the seams their spec names, and each pull request shows each new test failing before the code.

The owner asked for unit, integration and end-to-end tests that go through public interfaces, one behaviour per test with a name that says what it proves, tests kept next to the code they cover, and shared helpers instead of duplicated test code. `STYLE.md` already set one integration-test binary per crate and behaviour names. It did not say where TypeScript and Python tests live, where helpers live, or how tests are written. Comparable projects agree on the shape:
- **Two places per package.** Cargo's book splits unit tests in `src` from integration tests in `tests/`. pytest recommends the `src` layout with tests outside the package, so they run against the installed package. Base UI colocates component tests.
- **One separately compiled helper crate.** uv moved its `TestContext` into a `uv-test` crate, Cargo has `cargo-test-support`, and Nushell has `nu-test-support`.
- **One harness for several languages.** Temporal and Pulumi drive every SDK's cases from one harness written in the core's language.

Decided in [Test layout, naming, shared helpers and the test-first process](https://github.com/goodfetch/goodfetch/issues/277) ([grilling record](https://github.com/goodfetch/goodfetch/issues/277#issuecomment-6152108165)). The facts come from [Rust test tooling in 2026](https://github.com/goodfetch/goodfetch/issues/263) ([findings](https://github.com/goodfetch/goodfetch/blob/research/rust-test-tooling/rnd/research/2026-10-13-rust-test-tooling.md)), [Testing TypeScript and Python SDKs over a separate engine](https://github.com/goodfetch/goodfetch/issues/266) ([findings](https://github.com/goodfetch/goodfetch/blob/research/sdk-testing/rnd/research/2026-10-13-sdk-testing.md)) and [Testing a web console and a docs site](https://github.com/goodfetch/goodfetch/issues/265) ([findings](https://github.com/goodfetch/goodfetch/blob/research/web-console-testing/rnd/research/2026-10-13-web-console-testing.md)). The rules every test follows are in `TESTING.md`.

## Decision

### Levels

- **A test's level is the seam it crosses, not its speed.**
  - A **unit test** tests one module through its interface, in one process.
  - An **integration test** tests one package through its public interface, with real local stand-ins and real processes (ADR 0059).
  - An **end-to-end test** follows a user journey through a shipped surface.
  - Conformance suites stay what ADR 0032 makes them: interface tests of a contract with several implementations.
- Which suite (Quick, Pre-merge or Release) runs each level is decided by [Test suites, local commands and time budgets](https://github.com/goodfetch/goodfetch/issues/276). Where end-to-end tests live is decided by [Journey catalog: format, IDs, seeded data and surfaces](https://github.com/goodfetch/goodfetch/issues/278).

### Layout

| | Unit tests | Integration tests |
|---|---|---|
| Rust crate | `#[cfg(test)] mod tests` at the end of the module file, or in `<module>/tests.rs` | `tests/it/main.rs` and its modules, one binary per crate |
| TypeScript package, Console | `foo.test.ts` (or `.tsx`) beside `foo.ts` | `<package>/tests/`, importing only the package's own name |
| Python package | `tests/unit/`, mirroring the module paths under `src/` | `tests/integration/`, importing only the public package |

> Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no suite, test layout, Journey surface, security boundary, Performance budget, Feature kind (`screen:`) or actor exists for the Console.

- **A unit test reaches only the module's interface**: the items its callers use. It never calls a private function. Product code has no `#[cfg(test)]` branch and no `pub` item added for tests.
  > Amended by [ADR 0060](0060-distributed-and-crash-behaviour-is-simulated-on-tokio-model-checked-and-faulted-at-io-seams.md): the `sync` modules' `goodfetch_loom` switch is the one test-only `cfg` in product code.
- **A Rust crate splits its integration tests into named area harnesses** (`tests/<area>/main.rs`, with `autotests = false`) only when its pull request shows a localized test edit taking more than 5 seconds to rebuild with one harness. uv measured 7.52 s falling to 2.60 s with area harnesses, and per-file binaries cost more everywhere.
- **Tests of the `goodfetch` binary** live in the binary crate's `tests/it`, and find the binary through `CARGO_BIN_EXE_goodfetch`, never a guessed path under the build directory.
- **Python tests run with `--import-mode=importlib` and pytest 9's `strict = true`.** pytest's mirrored `tests/unit/` path is the nearest it supports without shipping tests in the wheel. The installed-wheel run of [SDK and CLI test stack](https://github.com/goodfetch/goodfetch/issues/285) can then run `tests/integration` unchanged.

### Writing a test

- **One behaviour per test**, through the interface. A test may make several assertions, as long as they check that one behaviour.
- **Arrange, act, assert, in that order, with one act.** No branch or loop computes an expected value. Expected values come from a source independent of the code: a literal, the spec, or the Proving ground's oracle.
- **A failure is asserted on its Error code**, never on its message text (ADR 0038).
- **Names state the behaviour in glossary terms:**
  - Rust: `resumed_run_never_repeats_an_effect`, with no `test_` prefix.
  - Python: `test_resumed_run_never_repeats_an_effect`.
  - TypeScript: `describe("<interface under test>")` with `test("resumed run never repeats an Effect")`, nested at most two levels deep.
  - No `should`, no issue numbers, no `it_works`.
  - Integration-test modules and files are named by behaviour area (`tests/it/resume.rs`), not by source file.
- **Parameterized tests name every case:** rstest's `#[case::name]` in Rust, so nextest runs each case as its own test; Vitest's `test.for` with the case name in the title; pytest's `parametrize` with an `id` per case. A loop over cases inside one test is not allowed, since it stops at the first failure and hides the others.
- **No mocking of owned code, enforced by tools:**
  - `cargo-deny` denies `mockall`, `mockito`, `wiremock`, `faux` and `mry`. httpmock stays allowed for true externals.
  - oxlint's `vitest/no-restricted-vi-methods` refuses at least `vi.mock`, `vi.doMock`, `vi.importMock`, `vi.mockObject`, `vi.spyOn`, `vi.stubGlobal` and `vi.stubEnv`.
  - ruff's `TID251` bans `unittest.mock` and `pytest.MonkeyPatch`, which can be used without the fixture.
  - pytest-mock is in no dependency group, which CI checks against `uv.lock`, since its `mocker` fixture needs no import that `TID251` could catch.
  - The test-support pytest plugin fails the collection of any test that requests `monkeypatch` or `mocker`. That list includes fixtures requested indirectly, so a third-party plugin fixture that uses `monkeypatch` would fail every test using it and cannot be adopted.

  A double is a written adapter at a seam: the in-memory adapter of the protocol for code that is remote but owned, and an adapter replaying recorded exchanges for true externals (`STYLE.md`'s four categories, which move to `TESTING.md`).
- **The TypeScript and Python SDKs take their environment as an injectable value.** This is ADR 0062's own rule; ADR 0059 makes it only for Rust. Whatever an SDK reads from the environment, such as `GOODFETCH_ENGINE_PATH` (ADR 0024), it reads from an environment map that an option can replace, defaulting to the process's own. A test passes the map, so no test patches the environment, in either language.

### Shared helpers

- **One internal Rust crate, `goodfetch-test-support`** (`publish = false`, under `crates/`), holds every shared helper:
  - the per-test folder, `GOODFETCH_HOME` and cleared environment;
  - the seed, printed at test start (ADR 0059);
  - unique names for resources shared by the whole user;
  - reading an address a child process reports;
  - starting the Proving ground from a seed;
  - commands with the cleared environment applied;
  - the shared snapshot redactions and filters.
  > Amended by [ADR 0087](0087-each-product-is-an-executable-crate-that-links-only-its-crates-and-has-its-own-size-budget.md): one more test-only crate holds turmoil's network adapter, which implements `goodfetch-engine`'s network interface and so depends on it. Only integration tests (`tests/it`) take it: a unit test of `goodfetch-engine` would see two copies of the engine's types, and one rule for every crate is simpler. Every other shared helper stays in `goodfetch-test-support`.
  > Amended by [ADR 0125](0125-a-test-adapter-of-a-goodfetch-interface-is-its-own-test-only-crate-which-no-unit-test-of-a-crate-it-depends-on-takes.md): this exception is now a rule. Every test adapter that implements a Goodfetch interface, turmoil's network adapter (`goodfetch-simulation`) and the Scripted model (`goodfetch-scripted-model`) among them, is its own test-only crate: `publish = false`, a dev-dependency only, banned from each Product's executable, and never taken by a unit test of a crate it depends on. Helpers that work on values only stay in `goodfetch-test-support`.
- **It depends on no Goodfetch crate.** It works on values only: paths, environment maps, seeds, addresses, processes and settings. A crate it depended on could not use it without two copies of that crate's types, and values are what ADR 0059 injects. Product crates list it only under `[dev-dependencies]`, which CI checks. It is a crate under `STYLE.md`'s second reason, since the compiler enforces its dependency direction.
  > Amended on 2026-11-07 (owner decision of 2026-11-07): test helpers that need Goodfetch's contract, the Goodfetch home's layout, `PROTOCOL_VERSION` and the `hello` contract among them, live in one more test-only crate, `goodfetch-test-client`, which depends on `goodfetch-protocol` and `goodfetch-test-support` and on no other Goodfetch crate. Like ADR 0125's test adapter crates it is `publish = false`, taken only under `[dev-dependencies]` and banned from each Product's executable; `goodfetch-protocol` never takes it, so its tests never see two copies of its types, and the Proving ground never takes it. Those facts had been copied by hand into test support, `xtask`, the Kennel and 111 test paths, and one copy drifted into a bug: `upgrade-files` copied an older Daemon's tokens into the new major. The new crate gives tests their one source, `goodfetch-protocol`, while `goodfetch-test-support` still depends on no Goodfetch crate, so the Proving ground, which takes it, reaches nothing of what it judges (ADR 0032). `xtask dependency-direction` checks these rules from the change that creates the crate.
  > Note (agent decision under the owner's delegation of 2026-11-08), 2026-11-08: `goodfetch-test-support` and `goodfetch-conformance` compile the Proving ground's `proving-ground/src/interface.rs` with `#[path]`, across crates. The Proving ground takes no Goodfetch crate in its release build, and its callers can't take the Proving ground, so one shared file gives them one source with no crate only it would need. A renamed field fails to compile in the Rust that reads through these types; the rest fails loudly when it runs.
- **The test versions of `Clock` and `Entropy` belong to the handles' own interface.** Each is built by a named constructor, `Clock::paused(start)` and `Entropy::seeded(seed)`, in the crate that defines it. Neither adds a test-only dependency to the product crate, since the paused version reads Tokio's clock, which only a test runtime pauses; the tests that pause it need Tokio's `test-util` feature under `[dev-dependencies]`. `disallowed-methods` refuses both constructors, so each test module or integration-test root that calls them opts in with `#[expect(clippy::disallowed_methods, reason = "…")]` (`#![expect]` in `tests/it/main.rs`), never with a crate-wide or `cfg_attr(test, …)` attribute in product code. `main` builds only the system versions. The seed and the start time come from `goodfetch-test-support` as values.
  > Amended by [ADR 0071](0071-performance-budgets-are-counted-per-change-and-time-is-measured-on-the-reference-host.md): a bench target's root opts in the same way, once, naming ADR 0071.
- **Setup is written once, in Rust.** `goodfetch-test-support` also builds a `goodfetch-test-env` executable. It prepares a test's folder, home and environment, starts what it is asked to (the Proving ground with a seed), and prints one JSON description: paths, the environment map, the seed and the addresses. It tears everything down when its standard input closes.
  - `@goodfetch/test-support`, a private pnpm package in `test-support/typescript/`, gives Vitest fixtures through `test.extend`.
  - `goodfetch-test-support`, a uv workspace package in `test-support/python/`, gives a pytest plugin registered through the `pytest11` entry point.
  - Both packages call `goodfetch-test-env`, are never published, and duplicate none of its setup.
- **`goodfetch`'s `test-util` feature stays what ADR 0038 makes it:** test utilities for people who embed `goodfetch`. It is never Goodfetch's own helper home. What it holds is decided by [SDK and CLI test stack](https://github.com/goodfetch/goodfetch/issues/285), as are the SDK fakes of the engine. If those fakes are generated from the catalog file, they live in the two test-support packages.
- **Expected results of Proving ground pages come from its oracle**, never copied into test code.
- **A helper written twice moves into test support.**

### Fixtures and data files

- **Fixtures live in a `fixtures/` folder beside the tests that read them.** Data that several languages read lives once, in `test-support/fixtures/`.
- **A test copies a fixture into its own temporary folder before changing it.**
- **No fixture file exceeds 256 KiB.** Larger or repetitive data is generated from a seed, by the Proving ground or by a generator in test support. Git LFS is not used.
- **A generated fixture or a recorded exchange is committed with a `<name>.source.toml` file beside it.** It names the generator or the recording job, its version, the seed or the recording date, and the command that regenerates it. Secrets are redacted from recordings. When recorded model exchanges are made is decided by [Testing AI features and agent skills](https://github.com/goodfetch/goodfetch/issues/281).

### Snapshots

- **Keep the three tools:** insta in Rust, Vitest's snapshots in TypeScript, inline-snapshot in Python.
- **Inline for short values** (about 20 lines); files for longer ones: insta `.snap` files, Vitest's `toMatchFileSnapshot` or `toMatchSnapshot`, and inline-snapshot's `external()`.
- **Every varying value is redacted or filtered** (IDs, times, durations, temporary paths), through the shared settings in test support.
- **CI never writes a snapshot, and fails on a mismatched, missing or orphaned one.** insta runs with `--check --unreferenced reject`. Vitest already fails on all three in CI. inline-snapshot raises on an empty `snapshot()` and compares values on CI. Its update runs single-process, since under xdist it can neither create nor fix a snapshot.
- **Snapshots are changed only through the tool's update or review command**, never by hand. A snapshot change is reviewed as a change of expected behaviour.
- **Snapshots cover structured outputs:** protocol responses, CLI output, generated surfaces, extraction output. They never replace a precise assertion on one value.

### Test-first process

- **An agent implementing a ticket follows the `tdd` skill.** The seams the spec names in its Testing Decisions are the seams agreed in advance. A test at a seam the spec does not name is allowed only when the pull request names that seam and the reason, and the fresh review accepts it.
  > Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): there is no such exception: tests sit only at seams agreed as `tdd` says, and the seams in the spec's Testing Decisions count as agreed.
- **Every behaviour lands red, then green.** The pull request's Verification section lists each new test with the failure it showed before the code existed. Commits stay green, so `git bisect` and the "all green" check work on each one.
  > Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): the section is the `pr` skill's Evidence section.
- **A bug fix starts with a test that reproduces the bug.**
- **The fresh review checks the pull request against the checklist in `TESTING.md`.**
  > Amended by [ADR 0133](0133-agent-work-follows-the-matt-pocock-skills-as-written-and-the-fresh-review-is-retired.md): no fresh review exists; the checklist is a review standard the `code-review` skill applies.

## Considered Options

- **Levels by speed:** rejected. Speed is the suites ticket's axis, and a slow unit test is still a unit test.
- **Only integration tests in Rust:** rejected. Internal modules such as the Frontier's rules have interfaces worth testing without making them public.
- **Area harnesses or per-file binaries everywhere:** rejected. uv's measurement shows one harness is right until a crate's suite is large, and per-file binaries cost the most.
- **Python tests inside the package** (pandas): rejected. They ship in the wheel or need excluding, and cannot run against the installed wheel.
- **Loops over cases instead of named parameterized tests:** rejected. They hide every case after the first failure.
- **Mocking libraries for owned code** (`mockall`, `vi.mock`, `unittest.mock`): rejected. They couple tests to the implementation, against `STYLE.md` and the `tdd` skill.
- **`goodfetch`'s `test-util` feature as the helper home:** rejected. Internal crates would have to depend on the embeddable crate to use it.
- **A `test-support` feature on each crate** (Zed has one on 93 crates): rejected. ADR 0038 allows features only on `goodfetch`.
- **A public trait for `Clock` that test support implements:** rejected. It would make a seam public only for tests.
- **Setup rewritten in each language:** rejected. Three copies drift. Temporal and Pulumi drive every language from one harness.
- **Calling `xtask` for test setup:** rejected. `xtask` is the release tool, and tests would have to build it.
- **Git LFS for large fixtures:** rejected. Data generated from a seed is smaller, reviewable and needs nothing on clones or CI caches.
- **syrupy instead of inline-snapshot:** rejected. syrupy updates under xdist, but inline-snapshot already fails on CI as needed. Its single-process update is a local inconvenience, not worth losing inline review.
- **Separate red commits before the code:** rejected. They break `git bisect` and the "all green" check. The pull request lists the red evidence instead.

## Consequences

- **Adds to ADR 0032.** Its tree gains `test-support/`: `typescript/` and `python/` hold the private test-support packages, and `fixtures/` the data several languages share. The internal `goodfetch-test-support` crate joins `crates/`. ADR 0032's text is not overridden, so it carries no note.
- **Adds to ADR 0038**, so ADR 0037's amendment rule carries it to every area's spec:
  - the layout of unit and integration tests in every language;
  - `goodfetch-test-support` and its dependency direction;
  - the handles' named test constructors and their `disallowed-methods` entries;
  - the ban on mocking libraries for owned code.
- **Changes `STYLE.md`.** Its Tests section moves into `TESTING.md` and becomes a pointer. Its crate reasons name `goodfetch-test-support` under the second reason.
- **`TESTING.md` gains a contents list and these sections:** Levels and layout, Writing a test, Dependencies in tests, Shared helpers, Fixtures and data files, Snapshots, Property tests, fuzzing and doctests, Test-first process, and Review checklist. The line on flaky tests moves unchanged; [Flakiness policy and checks that are non-deterministic by nature](https://github.com/goodfetch/goodfetch/issues/280) owns it.
- **Changes area 1's spec** ([Spec: Repository, CI and release (V1 area 1)](https://github.com/goodfetch/goodfetch/issues/130)):
  - the top-level layout adds `test-support/`;
  - the workspace adds `goodfetch-test-support` and the two private packages;
  - CI checks that `goodfetch-test-support` appears only under `[dev-dependencies]` and depends on no Goodfetch crate;
  - CI enforces the mocking bans, the 256 KiB fixture limit and the snapshot checks.

  [Workspace skeleton and the "all green" pull-request check](https://github.com/goodfetch/goodfetch/issues/132) creates the empty `test-support/` folder, the crate and the two packages, with these checks.
- **[Deterministic simulation and crash testing for leases, fencing, groups and run state](https://github.com/goodfetch/goodfetch/issues/275)** builds on `Clock::paused` and `Entropy::seeded`.
- **The "Test infrastructure" spec** builds `goodfetch-test-support`, `goodfetch-test-env`'s JSON description and the two thin packages, and names the exact `just` recipes for snapshot updates, with the other helpers ADR 0059 lists.
- **[SDK and CLI test stack](https://github.com/goodfetch/goodfetch/issues/285) and the Client surfaces area's spec** design the TypeScript and Python SDKs to read their environment from a replaceable map, as decided above.
- **Changes ADR 0059's last consequence.** The test `Clock` and the seeded `Entropy` are not test-support helpers: they are the handles' named constructors, `Clock::paused` and `Entropy::seeded`, in the crate that defines the handles. `goodfetch-test-support` supplies only their seed and start time.
- **Adds to `STYLE.md`'s lint list:** `Clock::paused` and `Entropy::seeded` join `disallowed-methods`.
- **Property tests:** parsers and stateful rules get property tests (`proptest` in Rust, `hypothesis` in Python), and their regression files are committed, as `TESTING.md` records.

> Corrected on 2026-10-19 in [#370](https://github.com/goodfetch/goodfetch/issues/370): `STYLE.md`'s pointer names ADR 0067's security tests, and `TESTING.md`'s fuzzing line carries ADR 0067's wording (every decoder of input Goodfetch did not write itself), not the line that moved from `STYLE.md`.

> Corrected on 2026-10-19 in [#370](https://github.com/goodfetch/goodfetch/issues/370): Vitest's `unstubEnvs` and `unstubGlobals` (ADR 0064) are only a safety net under the bans of `vi.stubEnv` and `vi.stubGlobal`, as `TESTING.md` now says.

> Corrected on 2026-10-19 in [#370](https://github.com/goodfetch/goodfetch/issues/370): the line on flaky tests did not stay unchanged: ADR 0064 replaced it in place with its rule that a test is never retried, and `TESTING.md`'s attribution of its sections now says so.

> Corrected on 2026-10-19 in [#370](https://github.com/goodfetch/goodfetch/issues/370): `TID251` also bans `_pytest.monkeypatch`, the module `pytest.MonkeyPatch` comes from, since importing it from there escapes a ban on `pytest.MonkeyPatch` alone.
