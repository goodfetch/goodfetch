# Run state is a portable SQLite file; runs resume from checkpoints, not shared storage

Each run keeps its state (frontier, staged results, sessions, effect journal, usage) in its own SQLite database in WAL mode with a single writer, and resumes on another machine from a checkpoint: a consistent, self-contained copy of that database, written to a location the caller gives and reported over JSON-RPC with its size, digest, run epoch and journal position. The cloud stores checkpoints wherever it likes and never swaps anything inside the engine. This keeps the single SQLite writer and crash isolation that the SDK binding relies on, and the one-engine-per-run model of ADR 0001. Decided in [Adopt runtime and state decisions](https://github.com/Jbmanllr/automan/issues/70).

## Considered Options

- **Pluggable live storage (PostgreSQL) that the cloud swaps in**, the R&D's working view: rejected. Every state contract would need conformance tests on two implementations, and it brings back the multiple writers that ADR 0001 avoided. PostgreSQL stays possible as a sink for results.
- **One database per install holding every run**: kept as a selectable setting, not the default. Export, import and checkpoint produce the same per-run file in both layouts.

## Consequences

- By default an acknowledged commit survives a power cut: `synchronous=FULL`, many operations grouped per commit by the single writer, `fullfsync` on macOS (plain `fsync` there does not flush the drive cache), and a bundled SQLite. An opt-in fast mode (`synchronous=NORMAL`) is labelled everywhere as surviving a process crash only: an OS crash or power cut may roll back recent commits, without corruption. No crawler competitor fsyncs its run state by default; the durable-execution engines do.
- No separate log as the source of truth in V1 (rqlite's design): grouped commits close most of the speed gap, and a second store would double the recovery code and tests. The V1 benchmark measures both modes on the Linux reference host and may reopen this.
- Checkpoints are taken on the `checkpoint` and `migrate` signals, on a clean stop, and periodically at an interval the caller sets (off by default locally). Between checkpoints, the journal covers a crash on the same machine.
- After a machine is lost, the run redoes the work since its last checkpoint. Effects in the checkpoint's journal are checked before being repeated, and effects since the checkpoint are reported as uncertain.
- The reset gate of ADR 0001 runs in both layouts. In the per-install layout, a discarded run must leave no recoverable bytes of its secrets.
- Checkpoints and exports contain session secrets; how they are protected is decided in [Security and compliance model](https://github.com/Jbmanllr/automan/issues/73).
