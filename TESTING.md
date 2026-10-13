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

## SDK and CLI tests

Decided in [ADR 0069](docs/adr/0069-sdks-and-the-cli-are-tested-against-a-scripted-engine-and-the-binary-of-the-same-commit.md).

### SDKs

- **Unit tests run against the Scripted engine in-process**, through an in-memory transport; integration tests and Journeys run the `goodfetch` binary built from the same commit, given through `GOODFETCH_ENGINE_PATH`.
  _Why:_ the binary can't fabricate a refused `hello` or a crash mid-call, and a fake alone passes where the engine fails.
- **Scripts live only in `test-support/fixtures/engine-scripts/`**, never inline in a test. Every message in them must validate against the committed JSON Schema bundle.
  _Why:_ one validated corpus keeps the stand-in true to the engine for every SDK at once.
- **Process faults use `goodfetch-scripted-engine`**, reached as an Attached engine through `GOODFETCH_ENGINE_PATH` or as a Daemon through a runtime file in the test's Goodfetch home. No build of `goodfetch` carries a fault switch.
- **No process outlives its test:** the test-support fixtures fail a test that leaves an engine, Daemon or handler process alive after teardown.
- **Every Client conformance case in `conformance/clients/` is implemented on every surface it lists**, carries its case ID, and runs with an Attached engine and with a Daemon. A Quick-suite check fails on a missing implementation or an unknown ID.
- **Each runner writes a case log; only `goodfetch-conformance`'s writer turns it into a result document.** A declared case missing from the log is `unproved`.
- **Completeness tests** check that every catalog operation, Error code and event has its generated counterpart in each SDK.
- **Generated code is type-checked** by `tsc` with `skipLibCheck: false`, pyright strict and mypy `--strict`.
- **Mapping and decoding are property-tested** (fast-check, hypothesis): user-data values with any keys, `__proto__` included, come back unchanged.
- **Type tests:**
  - TypeScript: `*.test-d.ts` files with `expect-type`'s `expectTypeOf` and `@ts-expect-error`, compiled by `tsc --noEmit` against the built `.d.ts`. Not Vitest's `typecheck` mode, tsd or TSTyche.
  - Python: `tests/types/` files with `typing.assert_type` and `# type: ignore[code]`, under pyright strict with `reportUnnecessaryTypeIgnoreComment = "error"` and under mypy `--strict`.
- **Python:**
  - the test-support plugin's `anyio_backend` lists `asyncio` and `trio` explicitly, and a missing trio fails the session;
  - every Client conformance case runs on asyncio, on trio and through the sync API;
  - free-threaded builds run the whole suite with `PYTHON_GIL=0`, and fail if the GIL is on; the sync API's unit tests also run under pytest-run-parallel.
- **TypeScript runtimes:** Vitest runs on Node with the `forks` pool. Bun, Deno 2, `workerd` and Vercel's edge runtime run the `smoke/*.mjs` scripts (only `node:assert`), each spawned by a Vitest test.
- **Installed packages:**
  - npm packages install from Verdaccio with no uplink, with npm, pnpm, Yarn, Bun and Deno, into a path with a space and a non-ASCII character; `--omit=optional` must fail at first use with Goodfetch's error;
  - wheels install into a fresh environment (`uv pip install --no-index --find-links`, and `pip`) and run `tests/integration` unchanged, on glibc and Alpine;
  - the crate builds a temporary consumer from `cargo package`'s output.
- **Models are scripted only through the testing kit's Scripted model** (`goodfetch::testing`, `goodfetch/testing`, `goodfetch.testing`).

### CLI

- **The binary runs under assert_cmd and is snapshotted with insta**, through `goodfetch-test-support`'s helper. One snapshot holds stdout, stderr and the exit code.
- **Every `--json` test validates stdout against the command's schema** from the bundle, checks that stdout holds nothing else, and snapshots it.
  _Why:_ `--json` is under semver (ADR 0030), and only a schema puts it in the breakage check.
- **Warnings, hints, deprecations and the age notice never reach stdout.**
- **Every exit code in `goodfetch-protocol`'s table has a test through the binary.**
- **Human output is snapshotted under `NO_COLOR=1`, a fixed `COLUMNS`, `TERM=dumb` and a fixed locale.** The `--help` tree is one snapshot, and a unit test calls clap's `Command::debug_assert`.
- **Terminal-only behaviour is tested in a pseudo-terminal with expectrl:** colour, progress, and person-only prompts, which must refuse when stdin is not a terminal.
- **A command that runs the engine is tested in both modes**, as rstest cases `attached` and `daemon`, against one snapshot.
- **Client faults** (a refused protocol version, a dead PID, a dropped connection) come from the Scripted engine run as a Daemon.

### Where they run

| Suite | SDKs | CLI |
|---|---|---|
| Quick | unit tests, type tests, completeness and property tests, script validation, the case-list check | in-process unit tests, `--help` snapshot, `debug_assert`, exit-code table |
| Pre-merge | touched SDKs on Linux with the newest Node LTS and CPython: integration, Client conformance, Scripted engine process, one installed-package run | every `tests/it` case, both modes, pseudo-terminal, interrupts |
| Release | every Node line, Bun, Deno and CPython (free-threaded included) on Linux x64; oldest and newest Node and CPython on every other platform; every install path on every OS; free-threaded builds on every OS | everything, on every platform |
