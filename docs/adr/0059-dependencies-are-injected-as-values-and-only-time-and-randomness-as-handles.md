# Dependencies are injected as values, and only time and randomness as handles

Every external dependency of Goodfetch's code is injected, never reached through a global. Most are injected as **values**: data roots, listen addresses, the environment, a host-to-address map. Tests give them a real local stand-in: real SQLite and files in a temporary folder, real sockets on port 0, real child processes, the real OS keychain and service manager. Only the dependencies with no local stand-in are injected as **handles** that a test can drive: one `Clock` (wall time, monotonic time and sleeps) and one `Entropy` (seeded, non-secret randomness).

The owner required every external dependency (time, network, storage, system services) to be injected so tests can replace it. `STYLE.md` says "Concrete types first; a trait only where two adapters exist", and area 3's spec says "Time-dependent cases set short durations through settings rather than mocking clocks". The two conflict only if *injected* means *behind a trait*. Comparable projects do both:
- **Values for what has a stand-in.** Cargo's test harness passes `HOME` and `CARGO_HOME` to each child process, and nextest's "process-per-test" isolation does the rest.
- **Handles only for what has none.** Materialize injects wall time with `NowFn`, TigerBeetle with a `Time` interface, and rustls with a `TimeProvider`. turmoil, madsim and proptest take a seed.

Tokio's paused clock covers monotonic time and timers in one current-thread runtime, but "the `Instant` in std continues to work as normal", and nothing reaches wall time in another process. libfaketime "cannot work with statically linked binaries", so it can't reach the static musl binary (ADR 0024), and the hardened runtime of ADR 0039 ignores `DYLD_` variables.

Decided in [Injection and determinism: time, randomness, storage, network and OS services](https://github.com/Jbmanllr/automan/issues/274), on the facts of [Deterministic time, randomness, storage and OS services in a multi-process Tokio system](https://github.com/Jbmanllr/automan/issues/264) ([findings](https://github.com/Jbmanllr/automan/blob/research/deterministic-testing/rnd/research/2026-10-13-deterministic-testing.md)). The rules each test follows are in `TESTING.md`.

## Decision

### Time

- **One `Clock` handle** is passed to every part of the engine and the Daemon that reads time. It has three operations: read wall time (UTC), read monotonic time, and sleep until a monotonic moment.
  - **System version:** reads the OS and `tokio::time`.
  - **Test version:** runs under Tokio's paused clock (`start_paused`). Monotonic time and sleeps are Tokio's own. Wall time is a fixed start plus the time the paused clock has advanced, so the two never disagree. `jump_wall(d)` moves wall time alone, which is how a test makes the Daemon see a machine wake from sleep (ADR 0036). No timer wheel is written for tests.
  - `std::time::SystemTime::now`, `std::time::Instant::now` and `tokio::time::Instant::now` are banned outside the `Clock` module by Clippy's `disallowed-methods`.
- **Rules that depend on time are pure functions that take "now" as an argument:** lease expiry (ADR 0003), a Schedule's next occurrence across DST (croner's explicit-start API, ADR 0036), backoff, and the Politeness store's rate math (ADR 0053). Their tests need no clock. Only the loops that sleep read the `Clock`.
- **Tests of the binary set short durations through settings** (ADR 0028 makes every timeout a setting), and no build carries a test-only clock control. Every duration the engine waits on is a setting or derives from one. Interval Schedules "count elapsed time" (ADR 0036), so a binary-level Schedule test uses a short interval. Cases that need a default-length lease, a day boundary or a DST date are tested in-process through the `Clock`.
- **Clocks outside Goodfetch's code stay real:**
  - The Valkey server's clock stays the single time source for every engine sharing the Politeness store. Its math is tested as a pure function, and the real store with short durations.
  - Chromium keeps its own clock. Proving ground pages take their time step from the task (area 2's spec).
  - The Script sandbox keeps `Date.now` and `Math.random` live (ADR 0051), and its tests compare the order of Effects.

### Randomness

- **One `Entropy` handle** serves every random *choice*: Schedule and backoff jitter, Session pool picks, fingerprint seeds, and the random part of ULIDs and Schedule IDs.
  - It uses a portable seeded generator, the `chacha20` crate. rand calls `StdRng` "non-portable" across versions, so a saved seed would not replay a run.
  - In production its seed comes from the OS, and the run state keeps it where replaying a run needs it (the fingerprint seed).
  - A test gets a fixed seed, which `GOODFETCH_TEST_SEED` overrides. The seed is printed when the test starts, so a test nextest kills on a timeout still shows it (madsim #217).
- **Security randomness never comes from `Entropy`.** Keys, tokens, the per-run data key (ADR 0015) and certificate authorities come from the OS generator (`getrandom`) and are never seeded.
- **Hash order never reaches output or ordering.** std `HashMap` "is randomly seeded". Clippy's `iter_over_hash_type` restriction lint flags loops over hash types, and ordered output uses `BTreeMap` or `IndexMap`.

### Storage

- **A `GOODFETCH_HOME` setting** roots one user's Goodfetch data, caches and configuration: run folders, the Run index, the Usage ledger, the Politeness file, the Schedule store, the Response cache, the browser cache and user configuration.
  - Unset, it means the platform's usual folders.
  - More specific settings still win: `GOODFETCH_BROWSERS_PATH`, `GOODFETCH_ENGINE_PATH` and `run_state.location`.
  - It is also a product feature: portable installs and per-job CI caches, as `CARGO_HOME`, `RUSTUP_HOME` and `DENO_DIR` provide.
  - Tests need it because on Windows `dirs` reads Known Folders, which a child's environment does not move.
- **No filesystem trait.** SQLite and files run for real in a per-test temporary folder, which is `STYLE.md`'s stand-in.

### Network

- **A `resolve` setting** maps host names to addresses, like curl's `--resolve`. It is applied wherever Goodfetch resolves a name: the engine's own resolver (ADRs 0024 and 0044), and Chromium's `--host-resolver-rules` wherever Chromium resolves on its own. Tests point every Proving ground host name at loopback, which a run allowlists for tests (ADR 0017). Users get staging hosts and pinned addresses from the same setting.
- **Proxies are tested against a real forward proxy** that the Proving ground serves. They are never mocked.
- **Local addresses (ADR 0044) are tested on real extra addresses:**
  - Linux: `127.0.0.0/8`, or a network namespace under sudo.
  - macOS: `lo0` aliases added during CI setup.
  - Windows: `127.0.0.x`.
- **Only scheduled live jobs, labelled as such, reach the public internet.** Every other job keeps ADR 0043's deny and its canary.

### OS services

- **Secret providers (ADR 0015):**
  - The keychain provider is tested against the real Keychain on macOS, the Credential Manager on Windows, and `gnome-keyring` started and unlocked on Ubuntu (keyring-rs's CI). Each test uses a unique service name.
  - Code that consumes Secrets uses the real environment or caller provider. `keyring-core`'s mock store is not used.
- **`daemon install`** (launchd agent, systemd user unit, Windows logon task) installs for real on each runner, as service-manager-rs's CI does. Generating the unit file is a pure function tested with snapshots.
- **Processes are always spawned for real.**
- **The environment is a value.** `std::env::set_var` is `unsafe` in Rust 2024, and `unsafe_code` is denied. The settings resolver (ADR 0028) takes an explicit environment map, and a child process gets its environment through `Command::env`.
- **The clipboard** is reached only by the Console, through Playwright's clipboard permissions. ADR 0033's refusal of the clipboard to Plugins is a sandbox test.

### Isolation

Any test can run beside any other:
1. One process per test (nextest).
2. A per-test temporary `GOODFETCH_HOME`.
3. A child process starts with a cleared environment whose `HOME`, `USERPROFILE` and `XDG_` variables point inside the test's folder.
4. Listeners bind port 0 and report their address, or use a Unix socket or named pipe in the test's folder. No fixed port.
5. One Valkey per CI job, with a namespace per test.
6. Resources shared by the whole user (keychain, service manager) run in a nextest test group, with unique names.
7. Every certificate authority is made for the test: the Proving ground's from its seed, a Daemon group's by `goodfetch group init` into the test's folder.

### SDKs and the Console

- **SDK durations and jitter are options.** Tests pass short values and a fixed seed. No Python clock mock reaches the engine's process, and time-machine 3 no longer mocks the monotonic clock. The TypeScript SDK may also use Vitest fake timers for timer logic that never reaches the engine.
- **The Console** uses Playwright's `page.clock`, which its docs recommend starting with `setFixedTime`, in end-to-end tests, and Vitest fake timers in component tests.
- **No SDK or Console test waits in real time** beyond a short duration it set itself.

## Considered Options

- **A trait in front of every dependency** (filesystem, network, environment, clock): rejected.
  - It spreads generics and slows compiles, which is `STYLE.md`'s reason for the two-adapter rule.
  - Its fakes pass where the real thing fails. A fake filesystem cannot carry SQLite's own C file I/O, and SQLite is what ADR 0002's durability claim is about.
- **Tokio's paused clock alone:** rejected.
  - It leaves wall time real, and Schedules, the Daemon's sleep detection and certificate expiry read wall time.
  - It only works on the current-thread runtime.
- **A hand-written test clock with its own timers** (TigerBeetle-style): rejected. Tokio's paused clock already provides the timers, so the `Clock` adds only wall time over it.
- **A test-only clock control in the binary** (Temporal's `TestService`, Neon's `testing` builds): rejected.
  - Tests would run a binary other than the shipped one, against ADR 0057's one binary.
  - It would need a Cargo feature outside the `goodfetch` crate, against ADR 0038.
  - A shipped binary must never expose clock skew. Everything it would reach is tested in-process instead.
- **libfaketime:** rejected. It cannot reach the static musl binary or a notarized macOS binary.
- **A deterministic simulator for the whole system** (turmoil or madsim) as the injection mechanism: rejected here. Both run a single process, and neither carries Chromium, sink processes or Script sandbox workers. Where they fit is decided by [Deterministic simulation and crash testing for leases, fencing, groups and run state](https://github.com/Jbmanllr/automan/issues/275).
- **`StdRng` for seeded randomness:** rejected. Its output changes across rand versions.
- **Fakes for OS services** (`keyring-core`'s mock store, a fake service manager): rejected. Every one runs for real on GitHub-hosted runners, and the real one is what users hit.
- **A per-test `HOME` alone, without `GOODFETCH_HOME`:** rejected. Windows Known Folders ignore it.

## Consequences

- **Amends `STYLE.md`.** "Concrete types first; a trait only where two adapters exist" now names `Clock` and `Entropy` as handles whose test version is the second adapter. Everything else is injected as a value with a real stand-in. The clock ban and `iter_over_hash_type` join its lint list.
- **Changes area 3's spec** ([Spec: Engine runtime (V1 area 3)](https://github.com/Jbmanllr/automan/issues/213)). "Time-dependent cases set short durations through settings rather than mocking clocks" becomes "In-process tests drive time through the injected `Clock`; tests of the binary set short durations through settings".
  - Leases (ADR 0003), the Daemon's idle stop, the governor's samples and checkpoints read the `Clock`.
  - Lease expiry is a pure function of the monotonic reading.
  - "A real Valkey container" holds on Linux runners. macOS installs Valkey with Homebrew, and Windows has no supported Valkey.
- **Adds to ADR 0028.** Two settings: `GOODFETCH_HOME` and `resolve`, both under semver from 1.0 (ADR 0030).
- **Adds to ADR 0038.** The `Clock` and `Entropy` handles, the clock ban and the value-injection rule apply to every crate. ADR 0037's amendment rule therefore carries them to every area's spec.
- **ADRs 0003, 0036, 0051 and 0053 are unchanged in behaviour.** Their time rules become pure functions over a reading, and their live clocks stay where they are decided.
- **[Engine tracer: an Attached engine answers `hello` over stdio](https://github.com/Jbmanllr/automan/issues/214)** starts its engine with a per-test `GOODFETCH_HOME` and a cleared environment. The `Clock` and `Entropy` arrive with the first code that reads time or chooses at random.
- **The "Test infrastructure" spec** builds the shared helpers: the test `Clock`, the seeded `Entropy`, the per-test home and environment, the port report, and the CI setup of `gnome-keyring` and `lo0` aliases.
