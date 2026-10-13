# Testing the TypeScript and Python SDKs over a separate engine

**Date:** October 13, 2026. **Ticket:** [Testing TypeScript and Python SDKs over a separate engine](https://github.com/Jbmanllr/automan/issues/266). Research input only: nothing here is a decision.

## Question

How do multi-language SDKs that drive a separate native process over a protocol test themselves, and which TypeScript and Python tools fit Goodfetch's SDKs?

Context from the repository: the SDKs are thin Clients that speak newline-delimited JSON-RPC to the `goodfetch` binary, as an Attached engine over stdio or a Daemon over a socket (ADR 0024). Their protocol code is generated from the committed JSON Schema bundle (ADR 0038). `STYLE.md` already sets Vitest on every Node line plus a Bun and Deno 2 smoke suite; pytest with the anyio plugin (asyncio and trio), hypothesis and inline-snapshot; pyright strict plus a mypy file; free-threaded CPython in CI. SDK conformance cases run in each surface's own test runner and write the shared result document (ADR 0032).

**Method.** Primary sources only: repository files read through `gh api` on each project's default branch, official docs (mostly their source in the project's repository), the npm and PyPI registries, and issue trackers. Versions and dates come from the registries unless stated. Heads read: Stagehand `bc9fc3b` (2026-10-12), playwright-python `222a2e0` (2026-10-12), Playwright `a3401cf` (2026-10-12), Temporal sdk-python `dca3a00` (2026-10-12), sdk-typescript `c42ad42` (2026-10-09), features `e45d681` (2026-10-13), Dagger `0da36c8` (2026-10-13), Pulumi `2255563` (2026-10-09), LanceDB `3837b7a` (2026-10-13), Polars `349d619` (2026-10-13), anthropic-sdk-typescript `2979ed5` (2026-10-09). Claims marked **(unverified)** were not confirmed against a primary source.

## Summary: the facts a decision rests on

1. **Two ways to test an SDK over a separate process, and the projects mix them.** Stagehand v4 (Python, Go) and Pulumi's language SDKs unit-test the SDK against a fake engine that speaks the real protocol in memory, then run a small smoke test of the installed package against the real thing. Playwright's language ports, Temporal's SDKs and Dagger's "slow" tests run against a real engine. Every project that uses a real engine pins it exactly (Playwright `DRIVER_VERSION`, Temporal `DEV_SERVER_DOWNLOAD_VERSION`) or builds it from the same commit (Dagger).
2. **Cross-SDK parity suites exist in two shapes.** Temporal's `features` repository has one Go runner that runs a feature snippet per language against a dev server and compares the scrubbed history across languages and versions. Pulumi's language conformance tests have one Go harness that converts a shared program into each language, runs it through that language's host as a separate process, and snapshot-tests the generated code. Stagehand v4, the closest product to Goodfetch, has no shared behaviour suite across its three SDKs: its browser integration tests run in TypeScript only.
3. **Generated code is tested three ways:** a drift check in CI (Stagehand, Dagger, Temporal), completeness tests ("every registered protocol method has generated Python models", Stagehand), and type-checking the generated code with every checker users run (Playwright runs mypy on `_generated.py`; Pulumi runs its Python conformance variants under mypy and pyright).
4. **TypeScript tooling as of today.** Vitest 5.0.4 (5.0.0 on 2026-09-03) needs Node 22.12 or later and defaults to the `forks` pool. Its type tests run `tsc` and are still marked Experimental; on TypeScript 7.0.2 its own `Assertion` type loses `.not` (open issue #11144). tsd 0.33.0 (2025-08-05) bundles TypeScript 5.9 and has had no commit since. TSTyche added TypeScript 7 only in 8.0.0-beta.0 (2026-10-11), and it refuses 7.0 (needs 7.1). StrykerJS 10.0.0's Vitest runner reports almost every mutant as survived on Vitest 5 (open issue #6210).
5. **Testing the package as installed is common and cheap.** Stagehand packs the npm tarball into a temporary project and runs its Python smoke against `uv run --isolated --no-project --with dist/*.whl`. Playwright and Temporal serve their freshly built packages from a local npm registry. Anthropic's TypeScript SDK installs its tarball into small projects for Bun (`bun test`), Deno (`deno test`), several Node module systems and bundlers. `node:test` runs on Node, first-class on Deno, and partly on Bun.
6. **Python tooling as of today.** pytest 9.1.1 (9.0.0 dated 2025-11-05 in its changelog). The anyio plugin's default `anyio_backend` fixture is module-scoped and parametrized over the backends that import, so trio tests silently disappear when trio is not installed (since anyio 4.12.0). pytest-xdist 3.8.0 runs session fixtures once per worker. inline-snapshot cannot create or fix snapshots under xdist and falls back to plain comparison there and in CI; syrupy 6.1.1 supports xdist with file locks. time-machine 3 no longer mocks the monotonic clock; no Python time mock reaches a separate engine process. pyright leaves unused `# type: ignore` unreported even in strict mode, while mypy's `--strict` reports them.
7. **Free-threaded Python is supported unevenly.** pydantic-core and time-machine ship `cp314t` and `cp315t` wheels, but Polars only checks that it imports on 3.14t and runs no tests there. The free-threading guide recommends 3.14t over 3.13t, setting `PYTHON_GIL=0` so the GIL stays off, and job timeouts because hangs are more likely.
8. **What users complain about.** Version skew: a Playwright client must match its server exactly, and its Java port shipped after the Docker image ([#39989](https://github.com/microsoft/playwright/issues/39989)). In Dagger, a nested older client gets the newer session's API view ([#13701](https://github.com/dagger/dagger/issues/13701), open). Temporal users hit mismatched `@temporalio/*` versions ([#1113](https://github.com/temporalio/sdk-typescript/issues/1113)). Flaky subprocess teardown: Windows asyncio pipe teardown bugs ([cpython#149388](https://github.com/python/cpython/issues/149388)), Temporal reruns one known flaky error, and Vitest ships a `hanging-process` reporter. Slow suites: one Temporal test package is "about 60%" of the TypeScript test wall time, and Polars runs only its fast tests by default.

## 1. Prior art

### 1.1 What each project tests against

| Project | How the SDK reaches the native side | Engine in the SDK's own tests | Installed-package test | Cross-SDK parity |
|---|---|---|---|---|
| Stagehand v4 (TS, Python, Go) | Bidirectional JSON-RPC to a Chrome extension | Python: in-memory fakes (`RecordingRPCClient`, `QueueTransport`). TS: integration tests against real Chrome and the built extension | TS tarball into a temp pnpm project; Python wheel smoke via `uv run --isolated` | None found; schema generation plus completeness tests |
| Playwright ports (Python, Java, .NET) | Bundled Node driver over a pipe protocol | Real driver and real browsers | Wheel installed into a fresh venv, then a client script runs | Each port pins `DRIVER_VERSION` and regenerates its API from the core docs |
| Temporal (TS, Python over a Rust core) | In-process Rust core; gRPC to a server process | Real dev server, pinned; time-skipping test server as a second target | TS packages published to Verdaccio, samples built from it | `temporalio/features` runs per-language snippets via one Go runner |
| Dagger (9 SDKs) | GraphQL over HTTP to an engine started by the CLI | Unit: pytest-httpx and a faked `dagger session` subprocess. Slow: a dev engine built from the same commit | Release dry runs (unverified detail) | Per-SDK dev modules; codegen from the engine's introspection |
| Pulumi (Node, Python, Go, .NET, Java) | gRPC between the language host, the program and the engine | `MockEngine` and mock resource monitor gRPC servers in the test process | Not checked | Language conformance tests in Go, with snapshots |
| LanceDB, Polars | In-process native extensions (PyO3, napi) | Not applicable (no separate process) | LanceDB installs the built wheel, then runs pytest | Not applicable |

Sources and details below.

### 1.2 Stagehand v4: fakes in Python, real browser in TypeScript

- The protocol is bidirectional JSON-RPC; Zod schemas produce `stagehand.v4.json`, from which `just generate` produces the Python models and Go structs; "only `protocolVersion` ... gates runtime compatibility", with semver rules per bump ([packages/protocol/README.md](https://github.com/browserbase/stagehand/blob/main/packages/protocol/README.md)).
- Python tests use a hand-written `RecordingRPCClient` that returns canned responses and validates results with the generated models in strict mode ([tests/_support.py](https://github.com/browserbase/stagehand/blob/main/packages/sdk-python/tests/_support.py)), and the RPC client is tested over an in-memory `QueueTransport` ([tests/test_rpc_client.py](https://github.com/browserbase/stagehand/blob/main/packages/sdk-python/tests/test_rpc_client.py)): parse and invalid-request errors, error codes preserved, timeouts, transport close rejecting pending requests, cancellation not turned into a timeout.
- Generated code has completeness tests: `test_every_registered_protocol_method_has_generated_python_models`, `test_every_registered_notification_has_a_generated_python_params_model`, round-trips of wire values and defaults ([tests/test_generated_models.py](https://github.com/browserbase/stagehand/blob/main/packages/sdk-python/tests/test_generated_models.py)). A protocol test checks that every Zod schema has a matching exported type ([protocol-schema-type-sync.test.ts](https://github.com/browserbase/stagehand/blob/main/packages/protocol/tests/protocol-schema-type-sync.test.ts)).
- Version negotiation is unit-tested as a pure function: older patch accepted, newer server minor accepted, server minor below the client's requirement rejected as `protocol-server-too-old`, major mismatch rejected, prereleases matched exactly ([runtimeCompatibility.test.ts](https://github.com/browserbase/stagehand/blob/main/packages/sdk-ts/tests/runtimeCompatibility.test.ts)).
- CI ([.github/workflows/ci.yml](https://github.com/browserbase/stagehand/blob/main/.github/workflows/ci.yml)): a path filter decides which SDK jobs run; `git diff --exit-code` on the schema and `python scripts/generate.py --check` and `go run . --check` catch generated-code drift; Python tests run on one Python version (`.python-version` = 3.11) on Linux; "Python wheel smoke" runs `uv run --isolated --no-project --with dist/*.whl python scripts/smoke.py` against a real Chrome and a local HTTP server ([scripts/smoke.py](https://github.com/browserbase/stagehand/blob/main/packages/sdk-python/scripts/smoke.py)); browser integration tests run only for TypeScript, one job per test file, writing CTRF JSON reports; a Windows job runs only three Go process-management tests.
- The TypeScript package test packs the tarball with `pnpm pack`, installs it into a temporary consumer project whose path contains spaces, and imports the public API from it ([packageContract.test.ts](https://github.com/browserbase/stagehand/blob/main/packages/sdk-ts/tests/packageContract.test.ts)).
- Tooling seen: pytest 9.0.3, pytest-asyncio 1.3.0 with `asyncio_mode = "auto"`, pytest `strict = true`, `ty` 0.0.82 as type checker, ruff, `uv_build` ([pyproject.toml](https://github.com/browserbase/stagehand/blob/main/packages/sdk-python/pyproject.toml)); Vitest and tsdown for TypeScript ([package.json](https://github.com/browserbase/stagehand/blob/main/packages/sdk-ts/package.json)).

### 1.3 Playwright's language ports: always the real driver

- playwright-python CI runs Linux, Windows and macOS × Chromium, Firefox, WebKit on Python 3.14, plus Chromium on 3.10 to 3.13 on each OS; sync and async suites run separately, each test with `--timeout 90`, each job 45 minutes ([ci.yml](https://github.com/microsoft/playwright-python/blob/main/.github/workflows/ci.yml)).
- "Test Wheel Installation" creates a venv, `pip install`s the built platform wheel, installs a browser into a temporary `PLAYWRIGHT_BROWSERS_PATH`, runs a client script and checks a screenshot exists ([tests/test_installation.py](https://github.com/microsoft/playwright-python/blob/main/tests/test_installation.py)).
- To "roll" a port, a maintainer changes the pin in `DRIVER_VERSION`, rebuilds, regenerates the API from a nearby checkout of the core repository, then runs `mypy playwright`: "every error in `_generated.py` points at an implementation method ... that lacks a new member or option" ([ROLLING.md](https://github.com/microsoft/playwright-python/blob/main/ROLLING.md)).
- `local-requirements.txt` pins pytest 9.0.3, pytest-asyncio 1.4.0, pytest-timeout 2.4.0, pytest-rerunfailures 16.7 and mypy 2.4.0 ([local-requirements.txt](https://github.com/microsoft/playwright-python/blob/main/local-requirements.txt)); the CI commands shown do not pass `--reruns`.
- The core repository's installation tests pack every package with `utils/pack_package.js`, serve them from a minimal local npm registry that "can serve local packages, or proxy to the upstream registry", install them in temporary workspaces with npm, yarn and pnpm, and compile `import { Page } from '<pkg>'` with `tsc` and `tsc --module nodenext` ([globalSetup.ts](https://github.com/microsoft/playwright/blob/main/tests/installation/globalSetup.ts), [registry.ts](https://github.com/microsoft/playwright/blob/main/tests/installation/registry.ts), [typescript-types.spec.ts](https://github.com/microsoft/playwright/blob/main/tests/installation/typescript-types.spec.ts)).

### 1.4 Temporal: real server per run, and a cross-SDK `features` repository

- `temporalio/features` holds "snippets for many Temporal features written in different Temporal SDK languages" plus "a runner and language-specific harnesses", to "ensure parity across SDKs", "confirm feature behavior across SDK versions" and "confirm history across SDK versions". The Go runner (`temporal-features run --lang LANG [--version VERSION]`) accepts a path to a local SDK checkout as the version, starts a dev server per run, and checks history three ways: replay of the run, replay of every stored history up to the current version, and an exact comparison of scrubbed histories. Bug regressions live under `features/bugs/<lang>` "with a version condition that shows in earlier versions it should fail" ([README.md](https://github.com/temporalio/features/blob/main/README.md)).
- Both SDK repositories call it on every pull request through a reusable workflow: `features-tests: uses: temporalio/features/.github/workflows/python.yaml@main` ([sdk-python ci.yml](https://github.com/temporalio/sdk-python/blob/main/.github/workflows/ci.yml)) and the TypeScript equivalent ([sdk-typescript ci.yml](https://github.com/temporalio/sdk-typescript/blob/main/.github/workflows/ci.yml)).
- sdk-python's own tests take `--workflow-environment` = `local`, `time-skipping`, `envconfig` or an address, so one suite runs against a local dev server, a time-skipping test server, Temporal Cloud or any server ([tests/conftest.py](https://github.com/temporalio/sdk-python/blob/main/tests/conftest.py)); the dev server download is pinned as `DEV_SERVER_DOWNLOAD_VERSION = "v1.8.3-server-1.32.0-162.0"` ([tests/__init__.py](https://github.com/temporalio/sdk-python/blob/main/tests/__init__.py)). The time-skipping environment downloads a test-server binary and "time skipping ... is global to the environment, not to the workflow under test" ([temporalio/testing/_workflow.py](https://github.com/temporalio/sdk-python/blob/main/temporalio/testing/_workflow.py)).
- sdk-python runs `pytest -n auto --dist=worksteal`, `timeout = 60` with `timeout_func_only`, and lints with pyright, mypy and basedpyright ([pyproject.toml](https://github.com/temporalio/sdk-python/blob/main/pyproject.toml)). CI covers Python 3.10 and 3.14 on Linux x64 and arm64, macOS arm64 and Windows, with `--reruns 3 --only-rerun "RuntimeError: Failed validating workflow"` on 3.10 only, a second pass on the time-skipping server (not on ARM), a job that regenerates protos and fails on a diff, a job on upgraded dependencies, and a Cloud job ([ci.yml](https://github.com/temporalio/sdk-python/blob/main/.github/workflows/ci.yml)).
- sdk-python tests its typing for false positives and false negatives: `tests/test_type_errors.py` collects every test file with comments like `# assert-type-error-pyright: '<regex>'` and runs pyright and mypy on it, checking that each expected error appears on the next line and that no other error appears (skipped on Windows) ([test_type_errors.py](https://github.com/temporalio/sdk-python/blob/main/tests/test_type_errors.py), [test_client_type_errors.py](https://github.com/temporalio/sdk-python/blob/main/tests/test_client_type_errors.py)).
- sdk-typescript CI: Node 20, Node 24 and Bun on Linux x64 and arm64, macOS arm64 and Windows x64, with Bun allowed to fail ("support is experimental"); it builds the packages, publishes them to a local Verdaccio registry, starts a Temporal CLI dev server at the version in `TESTS_CLI_VERSION`, runs the tests, then instantiates three sample projects from the Verdaccio artifacts ([ci.yml](https://github.com/temporalio/sdk-typescript/blob/main/.github/workflows/ci.yml)).

### 1.5 Dagger: fakes for unit tests, an engine from the same commit for the rest

- Python SDK unit tests fake the engine's HTTP with `pytest_httpx` (connection errors, bad responses, introspection errors) after setting `DAGGER_SESSION_PORT` and `DAGGER_SESSION_TOKEN` "to skip provisioning" ([test_connection_errors.py](https://github.com/dagger/dagger/blob/main/sdk/python/tests/client/test_connection_errors.py)); provisioning tests fake the `dagger session` subprocess with `pytest-subprocess`'s `FakeProcess`, including malformed stdout and a CLI too old to know `session` ([test_cli.py](https://github.com/dagger/dagger/blob/main/sdk/python/tests/provisioning/test_cli.py)). The anyio plugin is used, pinned to asyncio only ([conftest.py](https://github.com/dagger/dagger/blob/main/sdk/python/tests/conftest.py)).
- The Python dev module installs the client into a container attached to a Dagger engine built from the workspace (`dag.DaggerEngine(ws, ...).InstallClient(...)`), defines one test suite per supported Python (3.10 to 3.14), splits `Unit` (`-m "not slow and not provision"`) from `Slow`, and regenerates the client library from the engine's `IntrospectionJSON()` as a `+generate` check ([main.go](https://github.com/dagger/dagger/blob/main/.dagger/modules/python-client-dev/main.go), [test.go](https://github.com/dagger/dagger/blob/main/.dagger/modules/python-client-dev/test.go)).
- The TypeScript SDK is tested on Node's current and previous LTS (24.13.1, 22.22.0) and on Bun 1.1.38 ([ts-sdk.dang](https://github.com/dagger/dagger/blob/main/.dagger/modules/typescript-client-dev/ts-sdk.dang)).
- An open issue shows the version-skew risk in a client-engine design: "a program using the v0.21.7 Go client library inside a v1 session receives the v1 API view. Its generated queries may then fail"; the proposal ends with "Add an end-to-end test with an older client library running inside a newer session" ([#13701](https://github.com/dagger/dagger/issues/13701), 2026-07-21).

### 1.6 Pulumi: a fake engine for SDK tests, a conformance harness for languages

- Python SDK tests start in-process gRPC servers, `LanghostMockResourceMonitor` and `MockEngine`, that implement the engine's services "for use in tests" while the program runs in a language host ([test/langhost/util.py](https://github.com/pulumi/pulumi/blob/master/sdk/python/lib/test/langhost/util.py)).
- Language conformance tests express each program once in PCL; for each language the harness generates SDKs from test providers, converts the program to that language, runs it through "a compiled language host running as a separate process", checks assertions on the resulting state, and snapshot-tests the generated code (`PULUMI_ACCEPT=1` updates snapshots). Python runs three variants, using mypy in one and pyright in two; Node runs TypeScript and plain JavaScript ([pulumi-test-language/README.md](https://github.com/pulumi/pulumi/blob/master/pkg/testing/pulumi-test-language/README.md)).
- Pulumi also gives its users a fake engine: unit tests "cut this communication channel and replace the engine with mocks", which "answer those commands from within the same OS process" ([unit testing guide](https://github.com/pulumi/docs/blob/master/content/docs/iac/guides/testing/unit.md)). Temporal gives users the opposite: `WorkflowEnvironment.start_local()` and `start_time_skipping()` start real server binaries (§1.4).

### 1.7 LanceDB and Polars: in-process native extensions

These do not cross a process boundary, so they only inform the Python tooling.

- LanceDB installs the built wheel (`pip3 install ... $(ls target/wheels/lancedb-*.whl)[tests,dev]`), starts services with `docker compose up` for its `s3_test` tests, then runs `pytest -m "not slow" --durations=30` ([run_tests/action.yml](https://github.com/lancedb/lancedb/blob/main/.github/workflows/run_tests/action.yml)); doctests run separately, and pyright checks only an `include` list of files ([python.yml](https://github.com/lancedb/lancedb/blob/main/.github/workflows/python.yml), [pyproject.toml](https://github.com/lancedb/lancedb/blob/main/python/pyproject.toml)). Its Node package uses Jest 29 on Node 22, 24 and 26, plus an Apache Arrow version matrix ([nodejs.yml](https://github.com/lancedb/lancedb/blob/main/.github/workflows/nodejs.yml)).
- Polars uses `--import-mode=importlib`, `--dist=loadgroup` and `--max-worker-restart=0` because "a hard worker crash (e.g. a segfault) hangs under `--dist=loadgroup`", and by default deselects `slow`, `write_disk`, `hypothesis`, `docs`, `benchmark` and other markers ([py-polars/pyproject.toml](https://github.com/pola-rs/polars/blob/main/py-polars/pyproject.toml)). CI runs `pytest -n auto` on 3.11 to 3.14; for `3.14t` it only builds and runs `python -c 'import polars'` ([test-python.yml](https://github.com/pola-rs/polars/blob/main/.github/workflows/test-python.yml)).

### Fit for Goodfetch

- `STYLE.md` already classes the Daemon as "remote but owned": logic through an in-memory adapter of the protocol, real processes through the conformance suites. Stagehand's `QueueTransport` and Pulumi's `MockEngine` are that adapter in practice; both are hand-written, and Goodfetch could generate its fake engine's method table from the same catalog file the SDKs are generated from (an inference, not seen in any project).
- Every project that tests against a real engine pins or builds it from the same commit. For Goodfetch that is the binary built earlier in the same CI run, found through `GOODFETCH_ENGINE_PATH` (ADR 0024), never a downloaded release.
- ADR 0024's rule (a Daemon accepts the same major and the same or an older minor) is the kind of rule Stagehand tests as a pure function and Dagger's #13701 wants an end-to-end test for: an older SDK against a newer Daemon. Temporal's "path or version" runner argument is a model for running the conformance cases of SDK version N against engine N+1.
- ADR 0032 already places SDK conformance cases inside Vitest and pytest, writing one result document. Temporal and Pulumi use the other shape (one harness in the core language driving per-language snippets). This research does not argue against the recorded decision; it only notes that both shapes exist.
- Stagehand's completeness test ("every registered method has generated models") maps directly onto "every Operation catalog entry has an async method in both SDKs and a sync method in Python".

## 2. TypeScript tools

### 2.1 Versions checked (npm registry, 2026-10-13)

| Package | Latest | Published |
|---|---|---|
| vitest | 5.0.4 (5.0.0: 2026-09-03; 4.0.0: 2025-10-22) | 2026-10-12 |
| expect-type (behind `expectTypeOf`) | 1.4.0 | 2026-06-25 |
| tsd | 0.33.0 | 2025-08-05 |
| tstyche | 7.2.5 latest; 8.0.0-beta.0 on `next` | 2026-09-10; 2026-10-11 |
| typescript | 7.0.2 latest; `next` 7.1.0-dev | 2026-07-08 |
| @stryker-mutator/core, @stryker-mutator/vitest-runner | 10.0.0 | 2026-08-14 |
| publint | 0.3.25 | 2026-10-01 |
| @arethetypeswrong/cli | 0.18.5 | 2026-07-09 |
| verdaccio | 6.10.5 | 2026-10-03 |
| Bun | 1.4.3 | 2026-10-10 |
| Deno | 2.9.7 | 2026-09-17 |

### 2.2 Vitest 5: projects, pools, type tests

- Vitest 5.0 "requires Vite >= 6.4.0 and Node.js >= 22.12.0". Other breaking changes relevant to an SDK suite: inline projects inherit the root config by default, `testNamePattern` matches the `>`-joined full name, unawaited asynchronous assertions fail the test, fake timers also mock `Temporal`, and worker ids are 1-based ([migration guide](https://github.com/vitest-dev/vitest/blob/main/docs/guide/migration/index.md)).
- **Projects:** "multiple project configurations within a single Vitest process"; `workspace` "is deprecated since 3.2 and replaced with the `projects` configuration" ([projects.md](https://github.com/vitest-dev/vitest/blob/main/docs/guide/projects.md)).
- **Pools:** the default is `forks` (`child_process`); `threads` cannot use `process.chdir()`, and native libraries "have problems when running in multiple threads and run into segfaults"; `vmThreads` and `vmForks` are faster but leak memory and give native-module errors a different `Error` constructor ([pool.md](https://github.com/vitest-dev/vitest/blob/main/docs/config/pool.md)).
- **One process for all workers:** `globalSetup` runs "before the test workers are created" and can pass serializable data to tests with `project.provide()` and `inject()` ([globalsetup.md](https://github.com/vitest-dev/vitest/blob/main/docs/config/globalsetup.md)).
- **Type tests:** `*.test-d.ts` files are "only statically analyzed by the compiler"; "under the hood Vitest calls `tsc` or `vue-tsc` ... and parses results", and source errors fail the run unless `ignoreSourceErrors` is set ([testing-types.md](https://github.com/vitest-dev/vitest/blob/main/docs/guide/testing-types.md)). The `typecheck` options are marked Experimental; `typecheck.checker` accepts `tsc`, `vue-tsc` or any command producing `tsc --noEmit --pretty false` output ([typecheck.md](https://github.com/vitest-dev/vitest/blob/main/docs/config/typecheck.md)).
- **Under TypeScript 7:** "On TypeScript 7.0.2, `expect(value).not.toBe(...)` fails to typecheck: error TS2339: Property 'not' does not exist" (open, [#11144](https://github.com/vitest-dev/vitest/issues/11144), 2026-09-04). A Windows hang with typecheck in a test project is also open ([#9494](https://github.com/vitest-dev/vitest/issues/9494)).
- **Other runtimes:** Vitest's docs list Bun and Deno only as package managers, and warn Bun users to "use `bun run test` ... instead of `bun test`, otherwise Bun will run its own test runner" ([guide/index.md](https://github.com/vitest-dev/vitest/blob/main/docs/guide/index.md)); some module-runner features "do not work in Deno or Bun" ([experimental.md](https://github.com/vitest-dev/vitest/blob/main/docs/config/experimental.md)). No page says Vitest supports running tests on Bun or Deno.

### 2.3 Type tests: `expectTypeOf`, tsd, TSTyche, `@ts-expect-error`

- `@ts-expect-error` suppresses the next line's error, "but if there's no error, TypeScript will report that `// @ts-expect-error` wasn't necessary" (`Unused '@ts-expect-error' directive`) ([TypeScript 3.9 release notes](https://github.com/microsoft/TypeScript-Website/blob/v2/packages/documentation/copy/en/release-notes/TypeScript%203.9.md)). Anthropic's TypeScript SDK uses only this: a `shared/type-tests.ts` of `@ts-expect-error` lines, compiled by every ecosystem project "under that project's compiler and tsconfig" with `skipLibCheck: false` ([ecosystem-tests/README.md](https://github.com/anthropics/anthropic-sdk-typescript/blob/main/ecosystem-tests/README.md), [shared/type-tests.ts](https://github.com/anthropics/anthropic-sdk-typescript/blob/main/ecosystem-tests/shared/type-tests.ts)).
- tsd parses `.test-d.ts` files "for special constructs such as `expectError<Foo>(bar)`" and its assertions are strict ([readme.md](https://github.com/tsdjs/tsd/blob/main/readme.md)). tsd 0.33.0 depends on `@tsd/typescript ^5.9.2` (npm metadata); its repository was last pushed on 2025-08-05.
- TSTyche checks types with `expect(...).type.toBe<...>()` and can run against several TypeScript versions with `--target` ([README.md](https://github.com/tstyche/tstyche/blob/main/README.md)). TypeScript 7 support arrived in 8.0.0-beta.0 on 2026-10-11 ([CHANGELOG.md](https://github.com/tstyche/tstyche/blob/main/CHANGELOG.md)), and it refuses 7.0: "The installed TypeScript version 7.0 is not supported. Please upgrade to TypeScript 7.1 or later" ([StoreDiagnosticText.ts](https://github.com/tstyche/tstyche/blob/main/source/store/StoreDiagnosticText.ts)).

### 2.4 Testing the package as installed

- **Tarball into a temporary project:** Stagehand (§1.2); Anthropic's ecosystem tests install the `npm pack` tarball into small projects that cover runtimes, module systems, TypeScript configs, package managers and bundlers, against a local mock server. Each project runs from a temp copy "so nothing resolves from the repo's `node_modules`", installs with lifecycle scripts off, and a `knownFailure` field turns a failing step into `XFAIL` and an unexpected pass into a failing `XPASS` ([ecosystem-tests/README.md](https://github.com/anthropics/anthropic-sdk-typescript/blob/main/ecosystem-tests/README.md)).
- **Local registry:** Playwright's own registry server (§1.3) and Temporal's Verdaccio (§1.4). Both test unpublished packages with transitive dependencies as a user installs them.
- **Bun and Deno smoke runs:** Anthropic's Bun project runs `bun x tsc --noEmit` then `bun test`; its Deno project runs `deno test --no-check --allow-env --allow-net=127.0.0.1` then `deno check --all` ([bun/project.json](https://github.com/anthropics/anthropic-sdk-typescript/blob/main/ecosystem-tests/bun/project.json), [deno/project.json](https://github.com/anthropics/anthropic-sdk-typescript/blob/main/ecosystem-tests/deno/project.json)). Temporal runs its full suite on Bun but lets that job fail (§1.4); Dagger tests on Bun 1.1.38 (§1.5).
- **One test file for three runtimes:** Deno supports `Deno.test` and `node:test` "equally ... Neither is more supported than the other" ([runtime/test/index.md](https://github.com/denoland/docs/blob/main/runtime/test/index.md)), and its sanitizers catch "leaking async operations, resources, and surprise exits" ([sanitizers.md](https://github.com/denoland/docs/blob/main/runtime/test/sanitizers.md)). Bun marks `node:test` "partially implemented": the in-process API works under `bun test`, but `node:test/reporters`, snapshots and the `--test` CLI mode are missing. Bun's `node:child_process` also cannot pass a child's `stdout` or `stderr` as another child's stdio ([nodejs-compat.mdx](https://github.com/oven-sh/bun/blob/main/docs/runtime/nodejs-compat.mdx)). `bun test` is Jest-compatible "but not everything is implemented" ([test/index.mdx](https://github.com/oven-sh/bun/blob/main/docs/test/index.mdx)).

### 2.5 Stryker

- The Vitest runner forces a single thread, bail on first failure, its own per-test coverage analysis, and "only `threads: true` is supported"; incremental mode reuses results for unchanged code and tests ([vitest-runner.md](https://github.com/stryker-mutator/stryker-js/blob/master/docs/vitest-runner.md), [incremental.md](https://github.com/stryker-mutator/stryker-js/blob/master/docs/incremental.md)).
- On Vitest 5, `@stryker-mutator/vitest-runner` 10.0.0 "runs zero tests against every mutant that has per-test coverage"; one project's score fell from 47.36 to 2.96 because Vitest 5 joins test names with `' > '` (open, [#6210](https://github.com/stryker-mutator/stryker-js/issues/6210), 2026-09-04; fix pull requests #6214, #6220, #6247 open). The forced `threads` pool also segfaults on large test sets (open, [#6223](https://github.com/stryker-mutator/stryker-js/issues/6223)).

### Fit for Goodfetch

- Vitest 5's Node floor (22.12) matches ADR 0024's Node lines (22, 24, 26). Its default `forks` pool fits suites that start an Attached engine per test file. `globalSetup` plus `provide` fits one Daemon shared by every worker.
- The repository is on TypeScript 7 with a TypeScript 6 alias only for compiler-API tools (`STYLE.md`). Today that leaves: `@ts-expect-error` files compiled by `tsc` (works with any compiler), Vitest's `expectTypeOf` through `tsc` (Experimental, and its own types currently break on 7.0.2), TSTyche 8 beta (needs 7.1), and tsd (TypeScript 5.9 only, inactive since 2025-08). Which one runs on the exact compiler users run is the deciding fact.
- The Bun and Deno smoke suite of `STYLE.md` has a documented shape in Anthropic's ecosystem tests: install the packed tarball, run `bun test` and `deno test` on a few files, then type-check. A `node:test` file would run on all three, with Bun's gaps listed above.
- Stryker cannot measure a Vitest 5 suite until #6210 ships. That is a blocker today, not a judgement on mutation testing.

## 3. Python tools

### 3.1 Versions checked (PyPI, 2026-10-13)

| Package | Latest | Published | Note |
|---|---|---|---|
| pytest | 9.1.1 | 2026-06-19 | 9.0.0 dated 2025-11-05 in the changelog |
| pytest-xdist | 3.8.0 | 2025-07-01 | repository active (pushed 2026-10-12) |
| anyio | 4.15.1 | 2026-09-05 | |
| trio | 0.34.0 | 2026-08-11 | |
| pytest-asyncio | 1.4.0 | 2026-05-26 | |
| hypothesis | 6.169.0 | 2026-10-11 | |
| inline-snapshot | 0.36.1 | 2026-10-12 | |
| syrupy | 6.1.1 | 2026-09-13 | |
| time-machine | 3.5.1 | 2026-09-08 | `cp314t` and `cp315t` wheels |
| freezegun | 1.5.5 | 2025-08-09 | repository last pushed 2025-08-19 |
| respx | 0.23.1 | 2026-04-08 | |
| pytest-httpx | 0.36.2 | 2026-04-09 | |
| pyright | 1.1.414 | 2026-09-10 | |
| mypy | 2.4.0 (2.0.0: 2026-05-06) | 2026-10-01 | |
| basedpyright | 1.40.2 | 2026-10-04 | |
| ty | 0.0.86 | 2026-10-09 | |
| pydantic / pydantic-core | 2.14.1 / 2.50.1 | 2026-10-11 | pydantic-core ships `cp314t` and `cp315t` wheels |
| nox | 2026.8.17 | 2026-08-18 | |
| tox / tox-uv | 4.65.4 / 1.37.0 | 2026-10-12 / 2026-10-11 | |
| uv | 0.13.0 | 2026-10-09 | |
| mutmut | 3.8.0 | 2026-09-12 | |
| cosmic-ray | 8.7.0 | 2026-08-09 | |
| pytest-run-parallel | 0.10.0 | 2026-08-04 | |
| pytest-mypy-plugins | 4.0.3 | 2026-05-14 | |

### 3.2 pytest 9

- 9.0.0 added subtests in core, native TOML configuration under `[tool.pytest]` (`[tool.pytest.ini_options]` still works, not both), and a `strict` option that turns on `strict_config`, `strict_markers`, `strict_parametrization_ids` and `strict_xfail`, which the changelog says to enable only "if you use a pinned/locked version of pytest". It dropped Python 3.9 and turned `PytestRemovedIn9Warning` into errors. 9.1.0 (2026-06-13) added official Python 3.15 support ([changelog.rst](https://github.com/pytest-dev/pytest/blob/main/doc/en/changelog.rst)).
- pytest recommends the `src` layout so tests "can run against an installed version after executing `pip install .`", and `--import-mode=importlib` for new projects ([goodpractices.rst](https://github.com/pytest-dev/pytest/blob/main/doc/en/explanation/goodpractices.rst)).

### 3.3 pytest-xdist

- Distribution modes: `load` (default), `loadscope`, `loadfile`, `loadgroup` (by `xdist_group` mark), `worksteal` and `no` ([distribution.rst](https://github.com/pytest-dev/pytest-xdist/blob/master/docs/distribution.rst)).
- "Each worker process will perform its own collection", so a `session` fixture runs once per worker; xdist has no built-in way to run it once, and its docs show a lock-file recipe ([how-to.rst](https://github.com/pytest-dev/pytest-xdist/blob/master/docs/how-to.rst)). Workers must collect the same tests in the same order ([known-limitations.rst](https://github.com/pytest-dev/pytest-xdist/blob/master/docs/known-limitations.rst)).
- Temporal uses `-n auto --dist=worksteal` (§1.4); Polars `-n auto --dist=loadgroup --max-worker-restart=0` (§1.7).

### 3.4 The anyio plugin with trio

- Three ways to run async tests: `anyio_mode = "auto"` (since 4.11.0) or `--anyio-mode=auto` (since 4.15.0), the `pytest.mark.anyio` marker, or the `anyio_backend` fixture. Auto mode "does not work if `pytest-asyncio` is installed and configured to use its own `auto` mode" ([testing.rst](https://github.com/agronholm/anyio/blob/master/docs/testing.rst), [versionhistory.rst](https://github.com/agronholm/anyio/blob/master/docs/versionhistory.rst)).
- The plugin's own `anyio_backend` fixture is `@pytest.fixture(scope="module", params=get_available_backends())` ([pytest_plugin.py](https://github.com/agronholm/anyio/blob/master/src/anyio/pytest_plugin.py)). In 4.11.0 the params were `get_all_backends()`; from 4.12.0 they are only the backends that import successfully (checked at the 4.11.0, 4.12.0, 4.13.0 and 4.14.0 tags). The docs page still calls the fixture "function scoped". Async fixtures with a scope wider than `function` need a project-defined `anyio_backend` of the same scope.
- The plugin provides `free_tcp_port_factory` and related fixtures, which the docs call "a must" for parallel runs under xdist or tox.
- 4.15.0 fixed "free-threading compatibility issues" on 3.14t around `start_blocking_portal()` and `to_thread.run_sync()`; trio 0.34.0 fixed `to_thread.run_sync` workers "leaking spawner's Context on py314t+" ([trio history.rst](https://github.com/python-trio/trio/blob/main/docs/source/history.rst)). trio's PyPI classifiers stop at 3.14 and do not mention free-threading.

### 3.5 Clocks: time-machine against freezegun

- time-machine mocks `datetime.now()`, `time.time()`, `time.clock_gettime()` for `CLOCK_REALTIME` and related functions "at the C layer, replacing the function pointers", so it affects every import site; "Clock reads from C-level code outside of the standard library are unaffected" ([usage.rst](https://github.com/adamchainz/time-machine/blob/main/docs/usage.rst)).
- 3.0.0 (2025-11-18) removed mocking of `time.monotonic()`, which had caused "freezes in asyncio event loops" and broken test durations; the changelog says to mock `monotonic` with `unittest.mock` where it is imported instead ([changelog.rst](https://github.com/adamchainz/time-machine/blob/main/docs/changelog.rst)).
- freezegun "does a find-and-replace mock of all the places that the `datetime` and `time` modules have been imported", which takes time proportional to loaded modules and "can't affect C extensions" ([comparison.rst](https://github.com/adamchainz/time-machine/blob/main/docs/comparison.rst), time-machine's own comparison).
- Neither tool reaches another process. Temporal solves engine-side time with a server feature: a time-skipping test server whose skipping "is global to the environment" (§1.4).

### 3.6 HTTP mocks: respx and pytest-httpx

- Both mock `httpx` only. pytest-httpx fails a test at teardown when a registered response was never requested, and by default when a request had no registered response ([README.md](https://github.com/Colin-b/pytest_httpx/blob/develop/README.md)). RESPX mocks "HTTPX with awesome request patterns and response side effects" ([README.md](https://github.com/lundberg/respx/blob/master/README.md)). Dagger uses pytest-httpx because its SDK talks GraphQL over HTTP (§1.5).

### 3.7 Snapshots: syrupy against inline-snapshot

- inline-snapshot stores the snapshot as a literal in the test source. Under pytest-xdist it "will act as if `--inline-snapshot=disable` is set", and `disable`, "the default for CI runs", means `snapshot(x)` "will just return `x`": the comparison still happens, but snapshots cannot be created, fixed or reported. In an interactive terminal the default is `create,review`, and with `create`, `fix` or `review` "snapshot comparisons always return `True`" until the run ends ([limitations.md](https://github.com/15r10nk/inline-snapshot/blob/main/docs/limitations.md), [pytest.md](https://github.com/15r10nk/inline-snapshot/blob/main/docs/pytest.md)).
- syrupy writes `.ambr` files (or one file per snapshot), updates with `--snapshot-update`, and fails on unused snapshots unless `--snapshot-warn-unused` is set. It detects unused snapshots under xdist since 5.5.0 (2026-07-06), which "may now fail" suites that passed before. 6.1.1 (2026-09-13) added a file lock for concurrent amber writes under xdist; the changelog calls it "opt-in" while the README's option table says "Locking is enabled automatically under xdist" ([README.md](https://github.com/syrupy-project/syrupy/blob/main/README.md), [CHANGELOG.md](https://github.com/syrupy-project/syrupy/blob/main/CHANGELOG.md)). syrupy 6.x supports Python 3.10+ and pytest 8+.

### 3.8 Type tests: `assert_type` under pyright and mypy

- The typing documentation's approach: plain Python files that call `assert_type`, plus invalid lines marked `# type: ignore[code]`, checked with mypy `--warn-unused-ignores` ([quality.rst](https://github.com/python/typing/blob/main/docs/reference/quality.rst)). typeshed's test cases follow it and are checked by mypy through `tests/regr_test.py` and by pyright in CI ([REGRESSION.md](https://github.com/python/typeshed/blob/main/tests/REGRESSION.md)).
- pyright reports a failed `assert_type` as an error by default (`reportAssertTypeFailure`), but `reportUnnecessaryTypeIgnoreComment` is `"none"` in every mode, strict included, so an unneeded ignore passes silently unless enabled ([configuration.md](https://github.com/microsoft/pyright/blob/main/docs/configuration.md)). mypy's `--warn-unused-ignores` is a strict flag (`strict_flag=True` in [mypy/main.py](https://github.com/python/mypy/blob/master/mypy/main.py)). mypy 2.0 changed defaults such as `--local-partial-types` and `--strict-bytes` ([CHANGELOG.md](https://github.com/python/mypy/blob/master/CHANGELOG.md)).
- Temporal checks expected errors per checker with regexes (§1.4); Pulumi runs its generated Python under both checkers (§1.6).

### 3.9 Running the matrix: nox, tox or uv alone

- nox supports `uv` as a backend (`-db uv`, or `uv|virtualenv` as a fallback chain), "does not install anything by default, including pip", and can download Python interpreters through uv ([usage.rst](https://github.com/wntrblm/nox/blob/main/docs/usage.rst)).
- tox-uv "replaces `virtualenv` and pip with `uv` in your `tox` environments" and supports `uv.lock` ([README.md](https://github.com/tox-dev/tox-uv/blob/main/README.md)).
- uv alone: `uv run --isolated` "forces a fresh environment" ([CLI reference](https://docs.astral.sh/uv/reference/cli/#uv-run--isolated)). For 3.14+, uv may pick a free-threaded interpreter "if a free-threaded interpreter comes before a GIL-enabled build on the `PATH`"; `+gil` and `3.14t` select explicitly ([python-versions.md](https://github.com/astral-sh/uv/blob/main/docs/concepts/python-versions.md)).
- In the studied projects: Stagehand uses uv alone, Temporal uses uv plus poethepoet, Polars uses `uv pip` in CI, and LanceDB and Playwright Python use pip. None of them uses nox or tox.

### 3.10 Testing wheels as installed

- Stagehand: `uv run --isolated --no-project --with dist/*.whl python scripts/smoke.py` (§1.2). Playwright: a fresh `venv` and `pip install <wheel>`, then a script (§1.3). LanceDB: install the wheel, then run the whole test suite against it (§1.7). Temporal builds and tests Alpine wheels with cibuildwheel ([ci.yml](https://github.com/temporalio/sdk-python/blob/main/.github/workflows/ci.yml)).

### 3.11 Free-threaded runs

- The guide recommends 3.14t over 3.13t, which was "approximately 30% slower in single-threaded performance", and says CI will "likely need to manually pass `-Xgil=0` or set `PYTHON_GIL=0`" to keep the GIL off, plus job timeouts because "deadlocks and hangs are more likely". No environment marker for free-threading exists for dependencies ([ci.md](https://github.com/Quansight-Labs/free-threaded-compatibility/blob/main/docs/ci.md)). `sys._is_gil_enabled()` reports whether the GIL came back ([running-gil-disabled.md](https://github.com/Quansight-Labs/free-threaded-compatibility/blob/main/docs/running-gil-disabled.md)).
- For thread-safety, it points to pytest-run-parallel and pytest-freethreaded, which run existing tests in many threads, and to explicit multi-threaded tests with `ThreadPoolExecutor` and `threading.Barrier` ([testing.md](https://github.com/Quansight-Labs/free-threaded-compatibility/blob/main/docs/testing.md)). pytest-run-parallel adds `--parallel-threads`, `--iterations` and a `thread_unsafe` mark ([README.md](https://github.com/Quansight-Labs/pytest-run-parallel/blob/main/README.md)).
- Polars runs only an import check on 3.14t (§1.7).

### 3.12 Mutation testing: mutmut and Cosmic Ray

- mutmut "must be run on a system with `fork` support", so Windows needs WSL; it runs pytest; it can drop mutants that mypy or pyrefly reject, but not with pyright or ty, because mutating one method "can break the types of all methods" ([README.rst](https://github.com/boxed/mutmut/blob/main/README.rst)).
- Cosmic Ray runs any test command, either locally (mutating a copy of the code in place) or through HTTP workers, and records a session ([concepts.rst](https://github.com/sixty-north/cosmic-ray/blob/master/docs/source/concepts.rst)).

### Fit for Goodfetch

- Goodfetch's async core uses anyio (`STYLE.md`), so pytest-asyncio would compete with anyio's auto mode (Stagehand, Playwright and Temporal use pytest-asyncio because they are asyncio-only). Because the default fixture drops backends that fail to import, a CI check that trio is installed, or an explicit `anyio_backend` fixture, keeps "every async test runs on asyncio and trio" true.
- Session-scoped engine fixtures run once per xdist worker. That gives one Attached engine per worker, which matches ADR 0024's one engine per Client. A shared Daemon across workers needs the lock-file recipe or one Daemon per worker.
- `STYLE.md` names inline-snapshot for protocol payloads. Under xdist and in CI it still compares, but it cannot create or fix snapshots, so updating snapshots needs `-n0`.
- The SDKs' transports are stdio, a Unix socket, loopback TCP and WebSocket (ADR 0024), not httpx, so respx and pytest-httpx would not test the SDK's transport.
- Time mocks stop at the process boundary. Engine-side time (Schedules, timeouts, Politeness) can only be controlled by the engine itself, which is the role Temporal's time-skipping server plays.
- Free-threaded runs: pydantic-core already ships `cp314t` and `cp315t` wheels; the SDK's own code is pure Python. `PYTHON_GIL=0` and job timeouts are the guide's two concrete CI settings.
- mutmut's fork requirement excludes native Windows runners, and its type filter does not use pyright, the checker that gates CI in `STYLE.md`.

## 4. User pains

### 4.1 Version skew between an SDK and its engine

- Playwright: "When running tests remotely, ensure the Playwright version in your tests matches the version running in the Docker container" ([docker.md](https://github.com/microsoft/playwright/blob/main/docs/src/docker.md)). A user on the 1.58 image found `npx playwright` pulling 1.59 and could not move the Java client to 1.59 because "the java artifacts for 1.59.0 are not released yet" ([#39989](https://github.com/microsoft/playwright/issues/39989), 2026-04-01).
- Dagger: an older nested client receives a newer session's API view, so "its generated queries may then fail" ([#13701](https://github.com/dagger/dagger/issues/13701), open).
- Temporal: users "regularly face issues" from "version mismatches on `@temporalio/*` packages" and package hoisting; a `doctor` command was proposed in 2023 and is still open ([#1113](https://github.com/temporalio/sdk-typescript/issues/1113)).
- Stagehand's answer is a protocol version separate from package versions, negotiated at connection time and unit-tested (§1.2).

### 4.2 Flaky subprocess tests

- Windows asyncio pipe teardown: `_ProactorBasePipeTransport._call_connection_lost` can raise `OSError: [WinError 6] The handle is invalid`, seen in production on 3.13.12 and 3.13.13; the report lists related teardown issues still open, gh-83413 among them ([cpython#149388](https://github.com/python/cpython/issues/149388), 2026-05-04, closed 2026-05-09).
- Temporal reruns only one known error, `--only-rerun "RuntimeError: Failed validating workflow"`, and only on Python 3.10; it also depends on `pytest-flakefinder` (§1.4).
- Vitest's `hanging-process` reporter "displays a list of hanging processes ... preventing Vitest from exiting safely" ([reporters.md](https://github.com/vitest-dev/vitest/blob/main/docs/guide/reporters.md)); `teardownTimeout` defaults to 10 s ([teardowntimeout.md](https://github.com/vitest-dev/vitest/blob/main/docs/config/teardowntimeout.md)); `detectAsyncLeaks` reports async resources left open, at a large speed cost ([detectasyncleaks.md](https://github.com/vitest-dev/vitest/blob/main/docs/config/detectasyncleaks.md)).
- A crashed xdist worker can hang a `loadgroup` run, hence Polars' `--max-worker-restart=0` (§1.7).

### 4.3 Slow suites

- Temporal TypeScript: "`@temporalio/test` alone is about 60% of the test wall time", so CI runs it beside the other packages instead of after them ([ci.yml](https://github.com/temporalio/sdk-typescript/blob/main/.github/workflows/ci.yml)).
- Polars deselects slow and other costly markers by default; LanceDB and Dagger split `slow` from unit tests; LanceDB prints `--durations=30` (§1.5, §1.7).
- Stagehand runs only the SDK jobs whose paths changed, and splits browser integration tests into one job per file with `max-parallel: 20` (§1.2).

### Fit for Goodfetch

- ADR 0024 already answers part of the version-skew problem: an Attached engine runs the exact version its SDK ships with, and a Daemon refuses an incompatible Client with a clean error. What remains to test is the supported skew itself (older SDK against a newer Daemon of the same major), which is the case Dagger has no test for.
- The Python SDK starts an Attached engine over stdio with asyncio's Proactor loop on Windows by default, which is where CPython's open teardown bugs sit. ADR 0032's matrix runs Windows on the merge queue.
- `STYLE.md` already says a flaky test is quarantined with an issue and never retried silently; Temporal's narrow `--only-rerun` on one known error is the closest prior art, and it still reruns.

## Unverified

- Whether Vitest 5's type tests run correctly with TypeScript 7's native `tsc` beyond the open `.not` issue (#11144); no Vitest document states TypeScript 7 support.
- Whether Vitest itself can run on Bun or Deno as the runtime; its docs only cover them as package managers.
- Whether TypeScript 7.1 restores the compiler API that tsd and TSTyche need; TSTyche's message only implies it.
- Whether trio and anyio pass their own test suites on 3.14t; their changelogs fix free-threading bugs, but no support statement was found.
- Whether pytest-xdist 3.8.0 (2025-07-01) declares support for pytest 9.1 and Python 3.15; not checked in its metadata.
- Dagger's installed-package tests (release dry run details) and Pulumi's installed-package tests were not inspected.
- That mutmut 3 runs only pytest; its README describes only pytest.
- Whether Playwright's language ports share test names with the core suite; not checked.
- Whether syrupy's xdist file lock is on by default: its changelog and README disagree.
