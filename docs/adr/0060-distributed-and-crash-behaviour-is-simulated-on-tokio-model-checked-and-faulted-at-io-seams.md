# Distributed and crash behaviour is simulated on Tokio, model-checked in TLA+, and faulted at I/O seams

Leases and run epochs (ADR 0003), Checkpoints (ADR 0002), the writer lock (ADR 0031), Daemon groups (ADR 0055), Partitioned crawls (ADR 0054), the Politeness store's degraded shares (ADR 0053), the Usage ledger (ADR 0041) and Sink delivery (ADR 0026) are correct only if they stay correct under crashes, power cuts, partitions, clock skew and unlucky timing. Ordinary tests meet none of these on purpose, and nothing tested ADR 0002's power-cut promise. Each behaviour is therefore tested at every layer that can break it: a **Simulation** of several hosts on turmoil, with no runtime swap; model-based tests in one process with a crash VFS under SQLite; TLA+ models checked by TLC and validated against traces of the code; and real processes under kills, LazyFS and Toxiproxy. Faults enter only at I/O seams, never through failpoints.

No single tool covers Goodfetch: every Rust simulator runs one process, and none carries Chromium, sink processes, Script sandbox workers or SQLite's own C file I/O. Comparable projects layer their tools the same way:
- Amazon's ShardStore pairs reference models with a crash operation, Loom and Shuttle ([SOSP 2021](https://cdn.amazon.science/77/5e/4a7c238f4ce890efdc325df83263/using-lightweight-formal-methods-to-validate-a-key-value-storage-node-in-amazon-s3-2.pdf)).
- etcd model-checks its Raft library in TLA+ with trace validation ([etcd-io/raft](https://github.com/etcd-io/raft/blob/main/tla/README.md)), and its robustness suite adds failpoints, `kill` and LazyFS.
- SQLite tests crashes through a VFS that drops unsynced writes ([testing](https://sqlite.org/testing.html)).

Decided in [Deterministic simulation and crash testing for leases, fencing, groups and run state](https://github.com/Jbmanllr/automan/issues/275), answered by the agent on the owner's delegation, on the facts of [Deterministic time, randomness, storage and OS services in a multi-process Tokio system](https://github.com/Jbmanllr/automan/issues/264) ([findings](https://github.com/Jbmanllr/automan/blob/research/deterministic-testing/rnd/research/2026-10-13-deterministic-testing.md)) and [Rust test tooling in 2026](https://github.com/Jbmanllr/automan/issues/263) ([findings](https://github.com/Jbmanllr/automan/blob/research/rust-test-tooling/rnd/research/2026-10-13-rust-test-tooling.md)). It builds on ADR 0059's `Clock` and `Entropy`. The rules every test follows are in `TESTING.md`.

## Decision

### Which behaviour gets which layer

| Behaviour | Pure / proptest | Model-based, crash VFS | Simulation | TLA+ model | Real processes |
|---|---|---|---|---|---|
| Leases and fencing (ADR 0003) | expiry, renewal | yes | in group scenarios | run fencing | two-copies conformance |
| Checkpoint and resume elsewhere (ADR 0002) | | yes | handoff, zombie copy | run fencing | kills, LazyFS |
| At-least-once delivery, exactly-once commits | | yes | yes | run fencing | protocol conformance |
| Writer lock (ADRs 0031, 0037) | | | | | multi-process, every OS |
| Daemon groups and lead epoch (ADR 0055) | | | yes | lead epoch | real group, partitions, `kind` |
| Partitioned crawls (ADR 0054) | rendezvous ownership | | yes | Partition handoff | real group |
| Degraded shares (ADR 0053) | engine-side admission | | | degraded shares | real Valkey, cut-off engines |
| Usage ledger (ADR 0041) | slice sizing | yes | group slices | slices and reclaim | multi-process, kills |
| Sink delivery after crashes (ADR 0026) | | in-memory plugin adapter | | run fencing | sink conformance crash points |
| Power-cut safety of run state (ADR 0002) | | crash VFS | | | LazyFS |

### The Simulation: turmoil, with no runtime swap

- **turmoil**, pinned, runs several Goodfetch hosts (Daemons, Group members, engines) in one test process, each on its own paused current-thread Tokio runtime, over simulated TCP, with partitions, latency, message loss, host crashes and restarts, all drawn from one seed.
- **The engine stays on Tokio everywhere:** multi-thread in production, current-thread with the paused clock in tests (ADR 0059), one runtime per host in the Simulation. No crate depends on a replacement runtime.
- **What the code must do:**
  - **Sockets are opened only in the `transport` module.** Its interface listens and connects for the peer link, Client endpoints and the Politeness store, and takes the network as a value: the OS, the in-memory transport pair area 3 already exposes, or a test adapter. turmoil's adapter is test code at that seam. `tokio::net` and `std::net` socket types are refused elsewhere by `disallowed-types`.
  - **Blocking work goes through the `tasks` module, whose blocking policy is a value:** threads in production and in ordinary tests, inline in the Simulation, so SQLite takes no simulated time and ends at a deterministic point.
  - **The engine never builds its own runtime.** Only `main` and test helpers build one, and `tokio::task::block_in_place`, which panics on a current-thread runtime, is in `disallowed-methods`.
  - **Third-party time hooks read the `Clock`**, such as rustls's `TimeProvider` for certificate expiry.
  - **The Simulation builds with `--cfg tokio_unstable`**, so turmoil seeds each host's scheduler and `select!` order. The shipped binary does not.
- **Scenarios record a history** (each operation, its answer and each fault, in simulated time) and **invariant checkers** judge it:
  - **Fencing:** no output becomes visible under a stale lease epoch or run epoch, and every committed output is visible exactly once.
  - **One lead:** no two Daemons act as Lead Daemon under overlapping lead epochs, a Schedule fires at most once per occurrence, and one lead grants slices at a time.
  - **Limits:** charged plus outstanding Usage never exceeds a Limit, except the loss ADR 0055 allows after the last Group backup.
  - **Partitions:** every discovered URL lands in exactly one Frontier, and no domain is admitted by two Partitions at once.
  - **Audit log:** every event stays verifiable from the chain's head.
  - **Bounded liveness:** once faults stop, every run finishes or is resumable within a stated simulated time.
  The same checkers judge real-process tests and the conformance suites. Scenarios cover ADR 0055's VM path (majority confirmation, promotion, demotion, `group lead --confirm`), Checkpoint handoff on scale-down, `runs adopt`, a zombie copy writing to a fenced destination, Partition handoff, and clock skew through `jump_wall` on one host. Group scenarios use `politeness_sharing: per_daemon`, since the Politeness store is a real Valkey outside the Simulation.
- **Fault plans.** A **Fault plan** lists the faults and Client operations a test applies at named simulated moments. Plans are generated by proptest-state-machine, run by turmoil, and shrunk by proptest when they fail, so a failure is small, readable, and replays after the code changes, where a bare seed replays only on its own commit.
- **Determinism check.** A few seeds run twice and their histories and `tracing` output must match. A mismatch is a leak of real time, OS randomness or hash order, fixed at its source.

### TLA+ models with trace validation

- **Five models, checked by TLC** (tools pinned at 1.7.4): **run fencing** (leases, run epoch, destination compare-and-set by Guarantee class), **lead epoch** (ADR 0055's confirmation, promotion, demotion and membership change), **Partition handoff** (partition-map epochs, fenced domain handoff), **slices and reclaim**, and **degraded shares** (ADR 0053's shares and reservations over bounded time).
- **Trace validation.** Each transition a model names is a `tracing` event under one dedicated target, carrying the model's variable names as fields. A test subscriber writes them as NDJSON, and TLC checks each trace is a behaviour of the model, as etcd's Raft library does. No test build is needed.
- A model changes in the same pull request as the code whose behaviour it describes.

### Model-based tests in one process

- **proptest-state-machine** checks `run_state`, `lease`, `checkpoint`, delivery and the `ledger` against reference models, on real SQLite in a temporary folder and the test `Clock`. Transitions include handler timeouts (advancing the paused clock), late commits, crashes, power cuts, reopening and resuming. Sinks join through the plugin protocol's in-memory adapter with a test Sink of each Guarantee class.

### Crashes and power cuts

- **A crash VFS** under SQLite, built with `sqlite-plugin` and modelled on SQLite's own crash VFS: it holds writes until a sync, and at a crash point either keeps unsynced writes (a process crash) or drops, tears or reorders them from the seed (a power cut). It then refuses every later I/O of the crashed instance. It also injects I/O errors and a full disk at chosen operations. `run_state`, the Usage ledger and the Politeness file take the VFS name as a value when they open a database; production passes none. It runs on every OS.
- **LazyFS on Linux** under the real binary: the run folder and the Goodfetch home are mounted through it, the engine is killed, LazyFS's cache is cleared, and the run resumes. In default mode every acknowledged commit is present, and in both modes `integrity_check` passes, so fast mode loses commits but never corrupts.
- **Real process crashes** on every OS (`kill -9`, `TerminateProcess`) of engines, Clients, sink processes and Script sandbox workers, at seeded moments chosen from observed protocol events.
- macOS's `fullfsync` reaching the drive is invisible to all of these, so a test asserts the pragma is set.

### No failpoints

Faults enter only at I/O seams: the crash VFS for storage, the transport adapters and turmoil for the network, the plugin protocol's in-memory adapter for Sinks and handlers, Toxiproxy and process kills for real binaries. A crash only matters at an I/O boundary, and each one has a seam. A point that cannot be reached this way is I/O that bypasses its seam.

### In-process concurrency: loom

- **loom**, under the custom `--cfg goodfetch_loom`, checks every module that synchronizes threads with its own atomics, `Mutex`, `RwLock` or `Condvar`. Such a module imports them through its crate's `sync` module, which maps to loom under that cfg. A Goodfetch-specific cfg leaves dependencies that react to `cfg(loom)` unchanged, as crossbeam does with `crossbeam_loom`.
- Actors on Tokio channels are covered by the Simulation's seeded scheduling.

### Real-process faults: Toxiproxy and `kind`

- **Toxiproxy** sits between each real engine and the Politeness store, and between real Group members. ADR 0059's `resolve` setting maps each host name to that engine's Toxiproxy listener, so a test can cut off one engine or partition one Group member on any OS. Politeness checks:
  - requests in flight per key stay within the cap plus ADR 0053's minimum of 1 per cut-off engine;
  - degraded admission stops at `politeness_degraded_max`;
  - a reconnecting engine registers its requests in flight before the store frees its reservation.
  Valkey's clock stays real, so durations are short settings (ADR 0059).
- **ADR 0055's Kubernetes path** runs on `kind`: deleting and force-deleting the lead's pod, a lead cut off from the API server stopping within the lease period, and promotion by a `lead` change.
- **Sink delivery** crashes the engine at each delivery step through the crash VFS and the in-memory plugin adapter, and kills the sink process and the engine at seeded protocol messages in the real suite. The fencing checker judges by Guarantee class:
  - Fenced: no stale-epoch record is visible.
  - Record-fenced: no stale overwrite.
  - Keyed: no duplicate key.
  - At-least-once: every record is present at least once.

### Suites (assumed)

[Test suites, local commands and time budgets](https://github.com/Jbmanllr/automan/issues/276) owns the suites. This decision assumes four tiers, **quick**, **pre-merge**, **release** and **nightly**, and places each check:

| Check | Quick | Pre-merge | Release | Nightly |
|---|---|---|---|---|
| Pure time rules, proptest of decision logic | yes | yes | yes | |
| Model-based tests with the crash VFS | 32 cases, committed regressions | 256 cases | 256 cases, every OS | random seeds, 1 h |
| Simulation | committed Fault plans | committed plans, 16 seeds per scenario, determinism check on 4 seeds, one Linux job within 10 min | as pre-merge | random seeds, 2 h |
| loom | | changed crates, 2 preemptions, within 10 min | all, 2 preemptions | all, 3 preemptions |
| TLC | | when a model or a traced module changes: small constants within 10 min, trace validation of the Simulation's traces | all models | larger constants, 2 h |
| Real-process crashes, writer lock, shared per-user files | | Linux, fixed seeds | every OS | random seeds, every OS |
| LazyFS power cut | | | Linux x64 | Linux x64 |
| Toxiproxy cut-offs and partitions | | Linux, fixed seeds | yes | random seeds |
| `kind` lead scenarios | | | yes | yes |

A nightly failure of these checks is a bug, not flakiness, and no release candidate is cut while one is open. A budget overrun lowers case counts or seeds; it never removes a scenario.

### Regressions

1. A failing Simulation writes its shrunk Fault plan to a file and prints the command that replays it.
2. The fix commits that plan beside its scenario, with proptest's regression file. Committed plans run in quick and pre-merge.
3. A failure found by a non-deterministic layer (real processes, LazyFS, Toxiproxy, `kind`) is reproduced at the lowest deterministic layer that shows it, and that reproduction is the regression test. If none can, an issue extends the Simulation, and the failing seed joins the real-process scenario's committed seeds meanwhile.
4. A TLC counterexample becomes a Fault plan as well as a fix to the model or the design. A trace-validation failure is fixed in the code or in a stale model.
5. A test that once found a bug is never deleted.

## Considered Options

- **madsim:** rejected. It swaps Tokio for `madsim-tokio` and patches crates, so the whole engine rests on a fork whose releases have slowed (last 2025-10-11); RisingWave's simulation outgrew its CI.
- **Shuttle with `shuttle-tokio`:** rejected. Async code needs `tokio = { package = "shuttle-tokio" }` in every crate behind a Cargo feature, which is a runtime swap, and crates that link the real Tokio stay outside it. loom covers small primitives; the Simulation covers actors.
- **A hand-written simulator** (FoundationDB, TigerBeetle): rejected. turmoil already runs Tokio hosts on a simulated network.
- **Antithesis or Hermit:** not adopted. They alone run the real binary with Chromium deterministically, but both are Linux x86-64 only and Antithesis is a paid service with no public price. The layers above cover the rest; the owner can approve Antithesis later as a paid test resource.
- **A `Network` handle** beside `Clock` and `Entropy`: rejected. The transport is already a seam with two adapters (OS sockets and the in-memory pair), so turmoil is a third adapter there, and ADR 0059's two handles stay the only handles.
- **Failpoints** (`fail`, Neon's `testing` builds, etcd's `gofail`): rejected. `fail` is switched by a Cargo feature on the dependency, keeps a global registry, and scatters annotations through product code. A failpoint build of the binary is the test-only build ADRs 0057 and 0059 rule out.
- **Porcupine, Elle, Jepsen:** rejected. Goodfetch's properties are fencing, uniqueness and bounds, not linearizability of a key-value API, so invariant checkers over histories judge them. Jepsen needs a Clojure control node and Linux nodes; Toxiproxy gives real-process partitions on every OS, and `jump_wall` gives clock skew in the Simulation.
- **stateright or P for model checking:** rejected. stateright needs the code written as its actors, and P has no trace-validation precedent found. TLA+ is the established choice and has one in etcd.
- **No model checking:** rejected. A Simulation samples states and rarely reaches a promoted lead meeting a returning one across a membership change; a design flaw survives any number of tests of a correct implementation.
- **`kill -9` alone for crash safety:** rejected. The page cache survives a process crash, so it proves nothing about ADR 0002's power-cut promise.
- **dm-log-writes or dm-flakey; turmoil's `fs` shim:** rejected. Block-level replay needs root and a block device, and ShardStore's block-level variant "has not found additional bugs and is dramatically slower". SQLite's I/O goes through its C VFS, not `std::fs`.
- **Seeds alone as regressions:** rejected. A seed replays only on its own commit; a committed Fault plan stays meaningful after the code changes.

## Consequences

- **[Engine tracer: an Attached engine answers `hello` over stdio](https://github.com/Jbmanllr/automan/issues/214) builds on plain Tokio.** There is no runtime swap. From its first code it opens sockets only in `transport`, spawns and blocks only through `tasks` with a blocking policy value, and leaves runtime construction to `main`.
- **Adds to ADR 0038.** The `transport` seam with the network as a value, the blocking policy of the ownership tree, and the bans on sockets outside `transport`, on `block_in_place` and on building runtimes outside `main` and test helpers apply to every crate. ADR 0037's amendment rule therefore carries them to every area's spec.
- **Amends `STYLE.md`:** these rules join its concurrency and lint rules, the `goodfetch_loom` cfg joins its custom cfgs, and "Remote but owned" adds the Simulation.
- **Changes area 3's spec** ([Spec: Engine runtime (V1 area 3)](https://github.com/Jbmanllr/automan/issues/213)):
  - Its scenarios with several processes add Toxiproxy partitions and cut-offs, LazyFS, and seeded kill moments.
  - Its in-memory adapter scenarios move into the Simulation and the model-based tests above.
  - `run_state`, `ledger` and `politeness` open their databases with a VFS name value.
  - `group`, `partition`, `lease`, `ledger` and `politeness` emit the trace events their TLA+ models name.
- **Adds to ADR 0002:** its power-cut promise is now tested, by the crash VFS on every OS and by LazyFS on Linux.
- **Adds to ADRs 0003, 0026, 0053, 0054 and 0055:** their fencing, delivery, shares, handoff and lead-epoch rules each get a TLA+ model, invariant checkers and, except ADR 0053's store, Simulation scenarios. ADR 0026's sink suite keeps its cases and tiers, and gains crash points per delivery step and a verdict per Guarantee class.
- **The "Test infrastructure" spec** builds the shared pieces: turmoil's transport adapter, the history recorder and invariant checkers, Fault plans and their file format, the crash VFS, the LazyFS and Toxiproxy setups, the TLC and trace-validation jobs. Two facts are unverified here and are checked there: that `sqlite-plugin` registers a VFS with the SQLite that `rusqlite` bundles, and that LazyFS runs on GitHub-hosted Ubuntu runners (else in a container given `/dev/fuse`).
- **Owned by other tickets:** the suites and budgets ([Test suites, local commands and time budgets](https://github.com/Jbmanllr/automan/issues/276)), the layout of models, plans and loom tests ([Test layout, naming, shared helpers and the test-first process](https://github.com/Jbmanllr/automan/issues/277)), and how nightly failures are filed ([Flakiness policy and checks that are non-deterministic by nature](https://github.com/Jbmanllr/automan/issues/280)).
- **Delivery order (ADR 0058).** Each check lands with the code it covers. M1 brings the seams, the model-based tests, the crash VFS, LazyFS and the real-process crash tests. The Simulation's group and Partition scenarios, Toxiproxy and `kind` arrive with Daemon groups, Partitioned crawls and the Politeness store in M4.
