# Run state is a portable SQLite file; runs resume from checkpoints, not shared storage

Each run keeps its state (frontier, staged results, sessions, effect journal, usage) in its own SQLite database in WAL mode with a single writer, and resumes on another machine from a checkpoint: a consistent, self-contained copy of that database, written to a location the caller gives and reported over JSON-RPC with its size, digest, run epoch and journal position. The cloud stores checkpoints wherever it likes and never swaps anything inside the engine, which keeps the single writer and crash isolation that the SDK binding relies on, and the one engine per run of ADR 0001. Decided in [Adopt runtime and state decisions](https://github.com/Jbmanllr/automan/issues/70).

## Considered Options

- **Pluggable live storage (PostgreSQL) that the cloud swaps in**, the R&D's working view: rejected. Every state contract would need conformance tests on two implementations, and it brings back the multiple writers that ADR 0001 avoided. PostgreSQL stays possible as a sink for results.
- **One database per install holding every run**: kept as a local setting, not the default. It gives up one engine per run: a single local engine process writes for all of that install's runs, so the single writer holds but a crash stops them all. Automan Cloud always uses one database per run. Export, import and checkpoint produce the same per-run file in both layouts.
- **A separate log as the source of truth, with the database rebuilt from it** (rqlite's design): rejected for V1. It would double the recovery code and tests, for a speed gain that grouped commits can give at large batch sizes.

## Consequences

- By default an acknowledged commit survives a power cut: `synchronous=FULL`, many operations grouped per commit by the single writer, `fullfsync` on macOS (plain `fsync` there does not flush the drive cache), and a bundled SQLite.
- An opt-in fast mode (`synchronous=NORMAL`) is labelled everywhere as surviving a process crash only: an OS crash or power cut may roll back recent commits, possibly many, without corruption.
- None of the crawler libraries surveyed (Crawlee, Scrapy, spider-rs, Colly, Katana) fsyncs run state by default; Restate, DBOS, and Temporal and Hatchet on PostgreSQL do. The speed cost was measured on one Mac only, so the V1 benchmark measures both modes on the Linux reference host, and a large gap there may reopen the separate-log option.
- Checkpoints are taken on the `checkpoint` and `migrate` signals, on a clean stop, and periodically at an interval the caller sets (off by default locally). Between checkpoints, the journal covers a crash on the same machine.
- After a machine is lost, the run redoes the work since its last checkpoint. Effects in the checkpoint's journal are checked rather than repeated. Journal entries after the checkpoint are lost with the machine, so every effect the redone work reaches is checked first when it has a check, and otherwise reported as uncertain.
- The reset gate of ADR 0001 runs in both layouts. In the per-install layout, a discarded run must leave no recoverable bytes of its secrets.
- Checkpoints and exported run files contain session secrets: those fields stay encrypted under a key the caller holds, results Exports never carry them, and discarding a run destroys its key (ADR 0015).
