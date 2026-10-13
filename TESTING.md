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

## Simulation, model checking and crash testing

Decided in [ADR 0060](docs/adr/0060-distributed-and-crash-behaviour-is-simulated-on-tokio-model-checked-and-faulted-at-io-seams.md). Leases, run epochs, Checkpoints, the writer lock, Daemon groups, Partitioned crawls, degraded shares, the Usage ledger and Sink delivery are tested at every layer that can break them; the ADR's table says which.

### Code the tests rely on

- **Open sockets only in the `transport` module**, which takes the network as a value (the OS, the in-memory pair, or a test adapter). `tokio::net` and `std::net` socket types are in `disallowed-types` elsewhere.
  _Why:_ the Simulation swaps the network there, and nowhere else.
- **Block only through the `tasks` module**, whose blocking policy is a value: threads, or inline in the Simulation.
  _Why:_ a real blocking thread ends at an arbitrary simulated moment, so the run would not replay.
- **Never build a runtime in the engine.** Only `main` and test helpers build one; `tokio::task::block_in_place` is in `disallowed-methods`.
- **Give every third-party time hook the `Clock`** (rustls's `TimeProvider`).
- **Emit each transition a TLA+ model names as a `tracing` event** under the model's target, with the model's variable names as fields.
- **No failpoints.** Inject faults at I/O seams: the crash VFS, the transport adapters, the plugin protocol's in-memory adapter, Toxiproxy, process kills.
  _Why:_ a crash only matters at an I/O boundary, and each boundary already has a seam.

### Simulation

- **Several hosts in one process run on turmoil**, each on its own paused current-thread Tokio runtime, built with `--cfg tokio_unstable` so scheduling follows the seed. No crate swaps the runtime.
- **A scenario records a history** of operations, answers and faults, and **the shared invariant checkers judge it**: fencing, one lead, Limits, Partitions, the Audit log chain, and liveness within a stated simulated time once faults stop. Real-process tests and conformance suites use the same checkers.
- **Faults come from Fault plans** generated by proptest-state-machine and shrunk by proptest on failure.
- **Check determinism:** rerun a few seeds and compare histories and `tracing` output. Fix any mismatch at its source.
- **The Politeness store is not simulated.** Group scenarios set `politeness_sharing: per_daemon`.

### Model checking

- **The five protocols have TLA+ models checked by TLC** (pinned 1.7.4): run fencing, lead epoch, Partition handoff, slices and reclaim, degraded shares.
- **Trace validation:** TLC checks the NDJSON traces of Simulation and real-process tests against the model.
- **Change a model in the same pull request as the behaviour it describes.**

### Model-based tests and crashes in one process

- **Test `run_state`, `lease`, `checkpoint`, delivery and `ledger` with proptest-state-machine** against reference models, on real SQLite in the test's folder, with crash, power-cut, timeout, late-commit and resume transitions.
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
- **Cut off engines and partition Group members with Toxiproxy**, reached through each engine's `resolve` mapping. Keep Valkey's durations short.
- **Run ADR 0055's Kubernetes path on `kind`.**

### Suites

Placed by name; the suites themselves are decided by [Test suites, local commands and time budgets](https://github.com/Jbmanllr/automan/issues/276).

| Check | Quick | Pre-merge | Release | Nightly |
|---|---|---|---|---|
| Pure time rules, proptest of decision logic | yes | yes | yes | |
| Model-based tests with the crash VFS | 32 cases, committed regressions | 256 cases | 256 cases, every OS | random seeds, 1 h |
| Simulation | committed Fault plans | committed plans, 16 seeds per scenario, determinism check on 4 seeds, within 10 min | as pre-merge | random seeds, 2 h |
| loom | | changed crates, 2 preemptions, within 10 min | all, 2 preemptions | all, 3 preemptions |
| TLC | | when a model or traced module changes, within 10 min, with trace validation | all models | larger constants, 2 h |
| Real-process crashes, writer lock, shared per-user files | | Linux, fixed seeds | every OS | random seeds, every OS |
| LazyFS power cut | | | Linux x64 | Linux x64 |
| Toxiproxy cut-offs and partitions | | Linux, fixed seeds | yes | random seeds |
| `kind` lead scenarios | | | yes | yes |

- **A nightly failure of these checks is a bug, not flakiness.** No release candidate is cut while one is open.
- **Over budget, lower case counts or seeds; never drop a scenario.**

### Regressions

1. A failing Simulation writes its shrunk Fault plan to a file and prints the command that replays it.
2. The fix commits the plan beside its scenario, with proptest's regression file. Committed plans run in quick and pre-merge.
3. A failure from a non-deterministic layer (real processes, LazyFS, Toxiproxy, `kind`) is reproduced at the lowest deterministic layer that shows it, and that reproduction is the regression test. If none can, file an issue to extend the Simulation, and commit the failing seed to the real-process scenario meanwhile.
4. A TLC counterexample becomes a Fault plan and a fix to the model or design. A trace-validation failure is fixed in the code or in the stale model.
5. Never delete a test that once found a bug.
