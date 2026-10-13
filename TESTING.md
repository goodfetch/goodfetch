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
11. [AI features and agent skills](#ai-features-and-agent-skills)

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

## AI features and agent skills

Decided in [ADR 0065](docs/adr/0065-ai-features-are-tested-without-live-models-and-measured-by-oracle-graded-evaluations.md). Suites are the Quick, Pre-merge and Release suites of ADR 0061 (pending in [Test suites, local commands and time budgets](https://github.com/Jbmanllr/automan/issues/276)); Live checks are those of ADR 0064 (pending in [Flakiness policy and checks that are non-deterministic by nature](https://github.com/Jbmanllr/automan/issues/280)), in no suite. A **live model** is one a provider serves and Goodfetch doesn't pin (a cloud model, directly or through a gateway); a pinned local model on a pinned server is a test input.

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
