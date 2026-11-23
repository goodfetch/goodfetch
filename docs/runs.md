---
title: Runs
description: How a program's Run stops, finishes and resumes, and how a service closes it when the process is told to stop.
products: [collie, foxhound]
---

# Runs

This guide is for a person whose program starts a Run with `start()`, and for a service that must end cleanly when it is told to stop.

## Stopping and finishing

Leaving the scope or `close()` stops the Run with a Checkpoint. `finish()` ends it.

- **The scope stops.** TypeScript's `await using`, Python's `async with` and `with`, and dropping Rust's Run each stop a Run that still runs, `stopped` and `incomplete`, as `close()` does (ADR 0078). Starting its Run ID again resumes it from its last Checkpoint (ADR 0102).
- **Only `finish()` ends it finished.** Finishing also judges the Run's Verdict and promotes its Plan versions, so it is never a side effect of a block ending; and only Python's scope could tell a normal exit from an error (ADR 0117). Rust's `RunReport` is `#[must_use]`, so a program that drops `finish()`'s report is warned.
- **Closing is idempotent.** Closing again does nothing, and every later call on the Run, or on a value it opened, fails with `closed`.

## Signals

In a service, close the Run on `SIGINT` and `SIGTERM`: the SDK installs no signal handler, and a signal the program does not handle ends it as a crash would (ADRs 0078, 0093).

A process a signal ends at once leaves the Run to resume from its last Checkpoint. In a crawl, each request whose attempt was in flight then ends that attempt with `handler_lost`, counted against `crawl.retry.max` (ADR 0093). A `close()` before the process exits gives each such request back uncounted instead, and the Run resumes from a Checkpoint of that moment.

The SDK starts its Attached engine in a process group of its own, so a terminal's Ctrl+C reaches only the program, and the engine is still there when the program closes its Run (ADR 0078).

- **Python:** `SIGINT` raises `KeyboardInterrupt`, which unwinds through `with` and `async with` into `close()`. `SIGTERM` ends the process at once unless the program handles it, for example with a `signal.signal` handler that raises an exception the scope unwinds through.
- **TypeScript (Node):** both signals end the process at once. Listen with `process.once("SIGINT", …)` and `process.once("SIGTERM", …)`, and close the Run in the listener before the process exits.
- **Rust:** both signals end the process at once. Wait for them with `tokio::signal::ctrl_c()` and `tokio::signal::unix::signal(SignalKind::terminate())`, and close the Run before `main` returns.
