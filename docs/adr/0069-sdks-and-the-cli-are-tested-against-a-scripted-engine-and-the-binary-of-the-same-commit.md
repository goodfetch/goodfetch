# SDKs and the CLI are tested against a Scripted engine and the binary of the same commit

Each SDK's hand-written runtime is unit-tested in-process against a **Scripted engine**: a stand-in that plays one script of protocol messages, from one corpus of scripts that a check validates against the committed JSON Schema bundle. Integration tests and Journeys run the `goodfetch` binary built from the same commit, with an Attached engine and with a Daemon. Process faults the binary can't produce (a refused `hello`, a crash mid-call, garbage on stdout) come from the same scripts played by a Rust executable. Every Client surface implements one declared Client conformance case list in its own runner. Installed packages are tested from a local registry and fresh environments. The CLI is tested through its binary with assert_cmd and insta, every `--json` output is validated against a schema generated from Rust types, and terminal-only behaviour is tested in a pseudo-terminal. Each SDK ships a public testing kit (a Scripted model and a test engine helper), and Goodfetch's own suites use it.

The owner asked for unit, integration and end-to-end tests through public interfaces, with every external dependency replaceable and the shipped artefacts tested. The SDKs are thin JSON-RPC Clients of a separate binary (ADR 0024), their protocol code is generated (ADR 0038), and `--json` output and exit codes are under semver (ADR 0030). Comparable projects do this:
- **Two ways to test an SDK over a separate process, mixed.** Stagehand v4's Python SDK and Pulumi test against a fake that speaks the real protocol, then smoke-test the installed package. Playwright's ports, Temporal and Dagger run a real engine, always pinned or built from the same commit.
- **Parity across languages.** Temporal's `features` runner and Pulumi's language conformance tests drive one shared definition into each language. Stagehand has no shared behaviour suite, but it tests that every registered method has generated models.
- **Installed packages from a local registry.** Playwright and Temporal serve their freshly built npm packages from one; Stagehand and LanceDB install the built wheel into a fresh environment.
- **CLI output in snapshots.** uv wraps assert_cmd with insta filters; Cargo snapshots its help.
- **Testing kits for users.** The Vercel AI SDK ships mock models in `ai/test`, Pydantic AI ships `TestModel`, Temporal ships a test server and Pulumi ships mocks.

Tool facts that decided between options: Vitest's `typecheck` mode is Experimental and its `Assertion` type breaks on TypeScript 7.0.2 (open [#11144](https://github.com/vitest-dev/vitest/issues/11144)); tsd bundles TypeScript 5.9; TSTyche supports TypeScript 7 only in a beta that refuses 7.0; expect-type's own CI checks against the TypeScript 7 compiler. anyio's default `anyio_backend` fixture silently drops trio when it isn't installed. insta-cmd had one release in two years. expectrl drives a pseudo-terminal on Linux, macOS and Windows (ConPTY); rexpect is Unix only.

Decided in [SDK and CLI test stack](https://github.com/Jbmanllr/automan/issues/285) ([grilling record](https://github.com/Jbmanllr/automan/issues/285#issuecomment-6152351756)), with the agent answering on the owner's delegation, on the facts of [Testing TypeScript and Python SDKs over a separate engine](https://github.com/Jbmanllr/automan/issues/266) ([findings](https://github.com/Jbmanllr/automan/blob/research/sdk-testing/rnd/research/2026-10-13-sdk-testing.md)) and [Rust test tooling in 2026](https://github.com/Jbmanllr/automan/issues/263) ([findings](https://github.com/Jbmanllr/automan/blob/research/rust-test-tooling/rnd/research/2026-10-13-rust-test-tooling.md)). It builds on ADRs 0061 (suites), 0062 (layout and test support), 0063 (Journeys), 0064 (no retries), 0065 (AI) and 0066 (upgrades). The rules every test follows are in `TESTING.md`.

## Decision

### Levels for an SDK

- **Unit tests** (Quick suite) test each module of the hand-written runtime through its interface, in one process. The engine side is an in-memory transport played by a Scripted engine script.
- **Integration tests** (Pre-merge suite) run the package against the real binary of the same commit, and against the Scripted engine as a separate process for faults the binary can't produce.
- **End-to-end tests** are the SDK variants of Journeys (`sdk-rust`, `sdk-typescript`, `sdk-python`; ADR 0063), against the real binary and the local Proving ground.

### The Scripted engine

- **A Scripted engine plays one script:** the messages it expects from the Client and the messages it sends, engine-to-Client handler requests and events included. It fails the test on any message it did not expect and on any step left unused, and records every message.
- **A script can** answer `hello` with any protocol version or refuse it with any Error code, send any response or Error code, stream events under credits, call an SDK handler, stall (the SDK's own short timeout ends the wait), close the connection, write bytes that are not JSON-RPC, or exit with a code. This is the interface [Upgrade and version-mismatch testing](https://github.com/Jbmanllr/automan/issues/287) assumes.
- **Scripts live once**, as files in `test-support/fixtures/engine-scripts/`. A Quick-suite check validates every message of every script against the committed JSON Schema bundle, so a script the engine could never send fails there.
- **Matching** is on the method and the canonical JSON of params. Request IDs are correlated by step, never written as literals.
- **The script format** is a Rust type in `goodfetch-test-support`, whose JSON Schema `xtask codegen` emits.
- **Players:**
  - `@goodfetch/test-support` and the Python `goodfetch-test-support` play a script over an in-memory transport, for unit tests;
  - `goodfetch-test-support` builds a `goodfetch-scripted-engine` executable that plays a script over stdio, a Unix socket or loopback TCP. An SDK reaches it as an Attached engine through `GOODFETCH_ENGINE_PATH`, or as a Daemon through a runtime file in the test's Goodfetch home.
- **The Rust client** in `goodfetch` keeps area 3's in-memory transport pair for its unit tests and uses `goodfetch-scripted-engine` for process faults.

### The real binary

- **SDK integration tests run the `goodfetch` binary built from the same commit**, given through `GOODFETCH_ENGINE_PATH`. Never a downloaded release, never a guessed path.
- **Each test has its own Goodfetch home** from `goodfetch-test-env` (ADR 0062); a Daemon test starts its own Daemon there.
- **No process outlives its test.** The test-support fixtures record every engine, Daemon and handler process a test starts, and fail the test if one is alive after teardown. Vitest's `hanging-process` reporter runs in CI.

### Client conformance and parity

- **The Client conformance suite** is declared once in `conformance/clients/`. Each case has an ID, the capability it proves, the surfaces it applies to (`sdk-rust`, `sdk-typescript`, `sdk-python`, `cli`, and the MCP surfaces if [MCP, framework adapter and Integration test stack](https://github.com/Jbmanllr/automan/issues/286) adopts it), and the Proving ground tasks or script it uses.
- **Its capabilities are the hand-written runtime's:** `hello` and the version rule; request correlation; `$/cancel` from `AbortSignal` or anyio cancellation; one error class per Error code; credits and backpressure; event resumption after a reconnect; file-backed bodies read locally and streamed; SDK handlers both ways; `workers=N`; Daemon discovery and dead-PID replacement; the Attached engine stopping with its Client; Experimental and deprecation warnings; the `run_moved` redirect (ADR 0055).
- **Each surface implements every case that applies to it in its own runner**, carrying the case ID the way ADR 0063 carries Journey IDs. Every case runs with an Attached engine and with a Daemon.
- **A Quick-suite check fails** when a case has no implementation on a surface it applies to, or a test carries an unknown case ID.
- **Generated API completeness**, per SDK, in the Quick suite: every catalog operation has its async method in both SDKs, its sync method in Python and its method on the Rust client; every Error code has its error class; every event has its type; operations ADR 0011 keeps off agent surfaces are absent where they must be.

### Result documents

- **Each runner writes a case log**: one JSON Lines entry per case run, with the case ID, capability, outcome and evidence (the oracle's answer, the scripted messages). A Vitest reporter, a pytest plugin and a Rust helper in the test-support packages write it.
- **`goodfetch-conformance`'s writer turns the case log and the declared case list into area 2's result document.** A declared case absent from the log leaves its capability `unproved`. Journey variants (`kind = "journey"`) go through the same writer.

### Generated code and types

- **Generated code is type-checked by every checker users run:** `tsc` with `skipLibCheck: false`, pyright strict and mypy `--strict`.
- **Encoding and decoding are property-tested**, with hypothesis in Python and fast-check in TypeScript. Values in user-data positions (extracted Records, headers, metadata maps) get arbitrary keys, snake_case, camelCase and `__proto__` among them, and must come back unchanged; declared fields must round-trip between wire and SDK names (ADR 0038).
- **TypeScript type tests** are `*.test-d.ts` files using `expect-type`'s `expectTypeOf` and `@ts-expect-error`, compiled by `tsc --noEmit` against the built `.d.ts` with `skipLibCheck: false`. An unused `@ts-expect-error` is an error.
- **Python type tests** are files under `tests/types/` using `typing.assert_type`, with `# type: ignore[code]` on lines that must fail. pyright strict runs them with `reportUnnecessaryTypeIgnoreComment = "error"`, and so does mypy `--strict`.

### Python's three APIs

- **The test-support pytest plugin defines `anyio_backend`** over `asyncio` and `trio` explicitly. If trio does not import, the session fails.
- **Every Client conformance case runs three ways:** async on asyncio, async on trio, and through the generated sync API.
- **The sync API's own unit tests** cover its loop thread starting once and stopping at close, its documented error when called inside a running event loop, a handler running while a sync call is pending, `KeyboardInterrupt` cancelling the call in flight, and no thread left after close.
- **Free-threaded builds run the whole suite** with `PYTHON_GIL=0`, and the session fails if `sys._is_gil_enabled()` is true. The sync API's unit tests also run under pytest-run-parallel.

### TypeScript runtimes

- **Vitest runs the TypeScript SDK's unit, integration and conformance tests on Node**, with the `forks` pool.
- **Smoke scripts** (`sdks/typescript/smoke/*.mjs`) import the installed package and use only `node:assert`. A Vitest test per runtime and script runs each one under Bun, Deno 2, `workerd` (Miniflare) and Vercel's edge runtime (ADR 0049), against a real Daemon. Area 1's deployment-target smoke runs reuse these scripts.

### Installed packages

- **npm:** Verdaccio serves the packed root and platform packages with no uplink, under the outbound deny. npm, pnpm, Yarn, Bun and Deno (`npm:` specifier) install them into temporary projects whose path holds a space and a non-ASCII character, then run the smoke scripts. An install with `--omit=optional` must fail at first use with Goodfetch's error naming the cause and the fix (ADR 0024). `publint`, `@arethetypeswrong/cli` and the type tests run against the packed tarball.
- **PyPI:** each wheel installs into a fresh environment with `uv pip install --no-index --find-links` and with `pip`, and the package's `tests/integration` runs against it unchanged. Linux runs in a glibc and an Alpine (musl) container.
- **crates.io:** `cargo package` builds the crate, and a temporary consumer project builds and runs against the packaged sources.

### The CLI

- **Tool:** the binary runs under assert_cmd and is snapshotted with insta, through a helper in `goodfetch-test-support` that applies the cleared environment and the shared filters. One snapshot holds stdout, stderr and the exit code of one invocation.
- **`--json`:**
  - every `--json` output is a type in `goodfetch-protocol`, emitted in the JSON Schema bundle. A command generated from an operation outputs that operation's result; hand-written commands (`daemon start`, `config explain`, `usage`) get their own types;
  - each command's test validates stdout against its schema with the `jsonschema` crate, checks that stdout holds exactly that document (or only JSON Lines of the command's declared event type), and snapshots it;
  - a deprecated and an Experimental command run with `--json` still give valid stdout: warnings, hints and the age notice go to stderr only.
- **Exit codes** come from one table in `goodfetch-protocol` mapping each Error code to an exit code; a table-driven test checks each through the binary.
- **Human output** is snapshotted under `NO_COLOR=1`, a fixed `COLUMNS`, `TERM=dumb` and a fixed locale. The whole command tree's `--help` is one snapshot, and a unit test calls clap's `Command::debug_assert`.
- **Terminal behaviour is tested in a pseudo-terminal with expectrl:** colour and progress appear only on a terminal, and a person-only prompt (`goodfetch certs accept`, ADR 0042) accepts an answer from a terminal and refuses with its Error code when stdin is not one.
- **Interrupts:** a running Crawl sent `SIGINT` (Unix) or `CTRL_BREAK_EVENT` (Windows) checkpoints and exits with its exit code; a second interrupt stops at once.
- **Both modes:** every test of a command that runs the engine has two named cases, `attached` and `daemon`, which must match one snapshot.
- **As a Client:** the Scripted engine, run as a Daemon, gives the CLI a refused protocol version (the `goodfetch daemon restart` hint), a dead PID in the runtime file and a connection dropped mid-command.

### The testing kit users get

- **Each SDK ships a testing kit**, public and under semver (ADR 0030): `goodfetch::testing` behind `goodfetch`'s `test-util` feature, the `goodfetch/testing` subpath export, and the `goodfetch.testing` module. This settles what `test-util` holds.
- **It holds:**
  - **a Scripted model** in its SDK-handler form (ADR 0033): registered as the Model provider, it answers call *n* with reply *n* (text, a tool call, structured output, a stream or a provider error), fails on a call beyond its script and on replies left unused, records every request, and reports Usage from its script;
  - **a test engine helper:** an Attached engine in a temporary Goodfetch home with a cleared environment, `offline` on, the caller's `resolve` map and short durations, removed when the test ends.
- **Goodfetch's SDK suites script models only through this kit** (the scripting ADR 0065 leaves to SDK suites), so the shipped kit is what they test.
- It never holds Goodfetch's own helpers, `Clock::paused` or `Entropy::seeded` (ADR 0062).

### Suites

| Suite | SDKs | CLI |
|---|---|---|
| Quick | unit tests on the in-process Scripted engine, type tests, completeness tests, property tests of the mapping, script validation, the case-list check | in-process unit tests, the `--help` snapshot, `debug_assert`, the exit-code table |
| Pre-merge | touched SDKs on Linux, newest Node LTS and newest CPython: integration tests, Client conformance with an Attached engine and a Daemon, the Scripted engine process, one installed-package run; Python on asyncio, trio and sync | every `tests/it` case in both modes, `--json` validation, pseudo-terminal tests, interrupts |
| Release | every package: every Node line, Bun, Deno and every CPython with its free-threaded build on Linux x64; the oldest and newest Node line and CPython on every other platform; smoke scripts and installed-package runs for every package manager on every OS, Alpine included; free-threaded builds on every OS | every case on every platform, paths with spaces and non-ASCII characters, Windows console output |

Skew against released versions stays ADR 0066's. No runtime or platform is allowed to fail (ADR 0064).

## Considered Options

- **The real binary only:** rejected. It can't refuse `hello`, crash mid-call or write garbage on demand without a test-only control, which ADR 0059 rules out, and the Quick suite starts no process.
- **A hand-written fake per SDK** (Stagehand, Pulumi): rejected. Three fakes drift from each other and from the engine; one validated script corpus can't.
- **One Rust fake process only:** rejected. The Quick suite starts no process (ADR 0061), so unit tests need an in-process player.
- **Responses validated by each player at load:** rejected. TypeScript has no generated runtime validators (ADR 0038 types only), so one Rust check over the corpus does it once.
- **One Rust harness driving per-language snippets** (Temporal's `features`, Pulumi): rejected. ADR 0032 already runs SDK cases in each surface's own runner; the declared case list and its check give the parity.
- **Each language writing the result document itself:** rejected. The rule that an unreached capability is `unproved` would be written three times.
- **One shared Daemon for all SDK tests:** rejected. ADR 0059 gives each test its own Goodfetch home.
- **Vitest's `typecheck` mode, tsd or TSTyche for type tests:** rejected today. The first breaks on TypeScript 7.0.2, tsd is pinned to 5.9, TSTyche's TypeScript 7 support is a beta that refuses 7.0. TSTyche is reconsidered once it supports the pinned TypeScript.
- **pyright alone for Python type tests:** rejected. It leaves unused ignores silent by default; mypy users need the types right too.
- **anyio's default `anyio_backend`:** rejected. It drops trio silently when trio doesn't import.
- **Each runtime's own test runner for Bun and Deno** (Anthropic's ecosystem tests): rejected. Bun's `node:test` is partial and `bun test` is not Vitest; plain scripts spawned from Vitest run the same on every runtime and report through one runner.
- **Installing the npm tarball directly:** rejected. It can't resolve the per-platform packages in `optionalDependencies` the way a registry does for users.
- **hypothesis-jsonschema for property tests:** rejected. Its last release was in February 2024.
- **insta-cmd, trycmd or snapbox for the CLI:** rejected. insta-cmd had one release in two years; trycmd and snapbox add a second snapshot format beside insta.
- **`--json` checked by snapshots only:** rejected. A snapshot shows a change; only a schema from Rust types puts `--json` into ADR 0030's breakage check.
- **rexpect for pseudo-terminal tests:** rejected. It is Unix only.
- **No testing kit for users, or one in Rust only:** rejected. Users of the AI SDK, Pydantic AI, Temporal and Pulumi get one; ADR 0011 promises parity across SDKs.

## Consequences

- **Changes `STYLE.md`.** "mypy checks a small file that exercises the public API" becomes mypy `--strict` over the generated code and the type tests; "a smoke suite of the built package on Bun and Deno 2" becomes smoke scripts of the installed package on Bun, Deno 2 and the edge runtimes. The rules live in `TESTING.md`.
- **Adds to ADR 0038:** every `--json` output is a type in `goodfetch-protocol` emitted in the JSON Schema bundle, and one exit-code table maps Error codes to exit codes. ADR 0037's amendment rule carries this to every area.
- **Adds to ADR 0030:** the testing kits (`goodfetch::testing`, `goodfetch/testing`, `goodfetch.testing`) are public API under semver, and the catalog diff covers `--json` output types.
- **Adds to ADR 0032:** `conformance/clients/` holds the Client conformance case list; Verdaccio serves packed npm packages to the installed-package runs.
- **Adds to ADR 0062:** `goodfetch-test-support` also builds `goodfetch-scripted-engine` and defines the script format; the two test-support packages hold the in-process players, the case-log writers and the `anyio_backend` fixture; `test-support/fixtures/engine-scripts/` holds the corpus.
- **Adds to ADR 0065:** SDK suites script models through the testing kit's Scripted model.
- **Settles the interface ADR 0066 assumes:** the Scripted engine answers `hello` with any version and Error code.
- **Changes area 1's spec** ([Spec: Repository, CI and release (V1 area 1)](https://github.com/Jbmanllr/automan/issues/130)): the release jobs' smoke installs gain the local-registry and fresh-environment runs above; the deployment-target smoke runs for edge Clients reuse the SDK smoke scripts.
- **Changes area 2's spec** ([Spec: Proving ground and conformance (V1 area 2)](https://github.com/Jbmanllr/automan/issues/152)): the harness builds a result document from a case log another runner wrote. Assumed by [`goodfetch-conformance` tracer: the harness, the result document and a self-test suite](https://github.com/Jbmanllr/automan/issues/169).
- **Area 11's spec** (Client surfaces) writes the Client conformance cases, the testing kits, the `--json` types and the exit-code table.
- **The "Test infrastructure" spec** builds: the script format and its validation check, the three players, `goodfetch-scripted-engine`, the case-log writers and the case-list check, the process-leak check, the CLI helper and its filters, the Verdaccio and fresh-environment install harness, and the matrix jobs.
