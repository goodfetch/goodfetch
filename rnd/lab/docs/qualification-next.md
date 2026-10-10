# Integrated qualification — closed on October 8, 2026

Retained objective: pursue together HTTP acquisition, browser interactions,
SQLite writing, durable execution and comparison of the complete Rust workflow.
The figures of October 7/8 remain exploratory evidence, not thresholds to beat.

Interfaces already requested and retained: public JSONL protocol for adapters/writers,
workflow acquisition → extraction → validation → bounded queue → commit → ACK,
independent oracle for records and browser effects. These interfaces cover
correctness and recovery; they prove neither durability after a power cut,
nor compatibility with all sites, nor a distributed cloud deployment.

## Gates of this continuation

- [x] Runtime: native Rust/Tokio coordinator, native HTTP acquisition, validation,
  bounded batching and durable ledger; Node comparison with common topology and SQLite.
  [30/30 cells, 3 blocks, sources/binaries unchanged](../results/runtime-native-sustained-2026-10-07T23-21-59-298Z-bf6ecb/summary.json).
  The Rust configuration is now shared via Arc; the requalification keeps 30/30 cells. The initial series remains available.
- [x] HTTP: longer repeated campaigns, parameterized concurrency and delay,
  native/Spider/Crawlee comparison with correctness, saturation and visible rejections.
- [x] Browser: Stagehand v4, batch, Playwright, Puppeteer and @crawlee/stagehand;
  native actionability checks without masking incidents with common waits,
  exact trusted effects and delay/overlay/mutation variants.
- [x] SQLite: batch size, real writers, concurrent readers and checkpoints,
  same artifact, explicit commit guarantees, independent re-reading.
- [x] Durability: Node/Rust ledger, replay, fencing, crash before/after commit and ACK,
  takeover by a new coordinator, false success rejected and bound respected.
  [42/42 recovery cells](../results/runtime-recovery-2026-10-07T23-37-26-683Z-fcaced/summary.json)
  and [57 tests without skip or failure](../reports/qualification-current-tests.log).
- [x] Metrology: startup/work/drain/oracle separated, raw quantiles, processes
  explicitly included, sources/binaries frozen and failures kept.
- [x] Delivery: reproducible commands, cross-cutting report, evidence and
  recommendations distinguishing observations, hypotheses and limits.

The gates are closed for the executed scope, with the comparators' native failures and the limits kept in the report. They do not prove the whole future architecture.

Cross-cutting evidence: [report](../reports/integrated-2026-10-08.md), [first complete suite](../results/qualification-integrated-2026-10-07T23-21-58-974Z-6b64b0/summary.json), [complete requalification](../results/qualification-current-2026-10-08T00-11-54-553Z-21d1f2/summary.json), [contract audit](../reports/integrated-verification.json), [source audit](../reports/integrated-source-audit.json), [reproduction](../reports/qualification-reproduce.md).

## Fixes arising from the campaigns, before closure

- [x] Share the immutable configuration of the Rust coordinator, then remeasure
  the same Node/Rust cells; keep the initial series.
- [x] Fix the two stability observations on real RAF frames,
  cover the mobile click via the public worker and requalify the guards.
- [x] Initialize the schema/WAL only once before opening the concurrent
  writers; add the W4 contract and rerun the 108 SQLite cells.
- [x] Test the other Spider HTTP variants without erasing the three loopback
  redirect refusals linked to its native SSRF check.


Corrected results: runtime 30/30, recovery 42/42, guards 162/162, SQLite readers 108/108 (3,456,000 rows) and Spider excluding loopback redirect 60/60. The redirect remains a refusal by the native SSRF policy; no workaround. The red/green diagnostics of the guard and of the W4 startup are linked from the report.
