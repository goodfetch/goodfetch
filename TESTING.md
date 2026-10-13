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
- **CI test builds use the `ci-test` Cargo profile** (`test` with `debug = "line-tables-only"`).
- **Every run writes JUnit XML**, local or CI. Every CI job uploads it, pass or fail, as `junit-<suite>-<tier>-<os>-<arch>[-<runtime>]-<shard>`, and a failing job writes a summary of its failed tests to its page.
