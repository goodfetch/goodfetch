# Native workflow and cross-cutting qualification

The lab has a **native Rust/Tokio** coordinator, with reqwest HTTP, fixture
extraction, validation by an independent HTTP oracle, a bounded result channel,
a separate synchronous writer, SQLite transactions and a persisted ACK. The Node
coordinator uses fetch in its own process and a separate Node writer. Both writers
load the same SQLite 3.53.4 library and the same generic C SQL binding;
the jobs/leases/receipts logic is implemented separately in Node and Rust.

This qualifies an end-to-end HTTP prototype. The regex parser is specific to the
lab's corpus. This code is not yet a universal scraper, an integrated Chromium
engine, or proof of recovery of external effects.

## Reproducing

Select Node 25.5.0, then from `/Users/jb/Dev/goodfetch/rnd/lab`:

```sh
pnpm shared-build
pnpm workflow-build
pnpm check
node --test --test-concurrency=1 tests/*.test.ts tests/integration/*.test.ts
node tools/qualification-suite.ts
```

The suite runs the domains **serially**: extended Node/Rust workflows,
integrated HTTP backends, recovery, browser actionability, SQLite
readers/checkpoints, HTTP transports and open arrivals. The configuration JSON files used
to relaunch each campaign are kept in its suite folder.

The individual campaigns use `pnpm runtime <config.json>`,
`pnpm actionability 3` and `pnpm sqlite-readers 3`. The results have unique names
in `lab/results`, with manifests, frozen sources, raw data and re-read states. The
older campaigns remain available; no evidence database is deleted.

## Verified guarantees

- The seeds create fresh data whose expected values the oracle keeps.
- Each acquisition passes the oracle before entering the writer.
- At most C jobs hold unacknowledged results; only one batch writes at a time.
- Results and the `committed` state belong to a common WAL FULL transaction.
- The ACK is a separate transaction; an identical receipt can be replayed.
- A replaced lease fences off the commits of the old worker; divergent data
  are refused on replay.
- Crashes before/after commit and during/after ACK are injected by SIGKILL.
- A new coordinator process is tested after claim or between commit and ACK.
- The final verification reopens the database from the harness and checks each record,
  the number of jobs, integrity and foreign keys.

The coordinator stop injection includes a 50 ms test pause after
the chosen event, in order to remove the race between the signal and the ACK. This
pause is removed before recovery. It is not enabled in normal measurements.

## Metrology and limits

One event thread coordinates each native pipeline. The HTTP transports, bindings,
JSON encoders and allocations remain different: these are comparisons
of implementations, not isolated properties of the language. The traces are sent
to the harness; the Node lane avoids a second retention of the events in the core.
The Rust configuration is shared via `Arc` between producers; the first
series, which copied the whole corpus per worker, and its binary are kept to
compare the effect of this fix.

Startup, work, drain and oracle are distinguished. The complete throughput with startup and
oracle is provided separately. Per-job latency starts at the first lease and
does not include its prior wait in the durable backlog. This backlog is finite,
submitted before the work is timed; continuous workflow admission remains
to be qualified. Open arrivals are currently tested in the acquisition lane.

The native workflow's resources cover the coordinator and its descendants;
fixtures and oracle are excluded. The macOS probe uses `proc_pid_rusage` V4 with
Mach conversion via `mach_timebase_info`, validated against `getrusage`. For an
already active coordinator, its initial counters are subtracted. CPU/I/O remain
sampled counters that can miss the end of a process. RSS sums
count some shared pages several times; short peaks can
be missed. The disk counters are attributed by the kernel, not proof
of persistence after a power cut.

The actionability tests separate **native primitives** and **primitives with the common
guard**: selector uniqueness, visibility, enablement, stability and hit-test. A
blocked action must be refused without effect; a successful click must hit the right
target with a trusted event. This guard covers the lab's fixtures, without
proving all of Playwright's actionability, frames, closed shadow roots or the races
possible after the last check.

The trials remain local on shared macOS arm64. Dedicated Linux, remote network,
TLS/HTTP2, proxies, independent site corpus, agents/LLM, heartbeat, migrations,
full disk and external effects require additional qualifications.

## Retained diagnostics

The first suite of October 8, folder
`qualification-integrated-2026-10-07T23-16-32-138Z-d5bec9`, was interrupted after
the discovery of a wrong unit in the first CPU probe. Its Mach counters
were treated as nanoseconds. It is excluded from the main selection;
its `interrupted.json`, its manifest and its frozen sources explain the discrepancy.

The harness also fixed a 400 ms shutdown timer that remained referenced
after the worker exited. The shutdown test checks that no referenced timer
artificially keeps the supervisor alive.
