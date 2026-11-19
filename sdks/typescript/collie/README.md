# @goodfetch/collie

Collie's TypeScript SDK package and its `goodfetch-collie` bin (ADRs 0090 and 0095). It re-exports [`@goodfetch/collie-client`](../collie-client/README.md) and adds `Collie`, the Connection.

```ts
import { Collie } from "@goodfetch/collie";

const collie = await Collie.connect();
const run = await collie.start();
const report = await run.finish();
await collie.close();
```

- `Collie.connect({ env, logger, signal, maxPendingCalls, maxQueuedWriteBytes, maxListenerQueue })` starts Collie's executable as an Attached engine and finishes `hello` before it returns: a missing executable fails with `engine_not_found`, a refused Product with `not_in_product` and a refused protocol version with `protocol_version_unsupported`, each with a `fix`. It reads `GOODFETCH_COLLIE_ENGINE_PATH` from `env` (the process's environment when left out), else the platform package `@goodfetch/collie-<platform>-<arch>`, and gives the engine exactly `env` as its environment; Foxhound's variable is never read. An aborted `signal` fails it with the signal's reason once the engine it started has stopped. Past `maxPendingCalls` calls waiting for their answer (10,000) or `maxQueuedWriteBytes` bytes waiting to be written (64 MiB), a new call fails at once with `connection_overloaded`. A Session's listener holding `maxListenerQueue` events it has not yet got (1,024) loses those that follow and, once it has got the ones it held, is removed and ends with `listener_events_lost`, logged at error level; the other listeners go on. Events come up to 16 at a time, so a `maxListenerQueue` below 16 can end even a listener that returns at once. A bound that is no whole number of 1 or more fails with `setting_invalid`.
- `collie.start({ runId, tags })` starts a Run; `run.finish()` answers its Run report; `run.close()` stops a Run still running. A Run's other members are the catalog's `run.` operations, generated: `run.newPage()` opens a `Page` in the Run's default Session, `run.newSession()` a `Session` with its own `newPage()`, and `page.locator(selector)` builds a `Locator` without a call. `page.goto(url)` answers the document's `Response`, a 404 included, or `null` when the navigation had none, and any other member whose result has one field answers that field's value (ADR 0099), such as `page.evaluate`'s value; `page.formats({ markdown: true })` reads the page. `close()` is idempotent on a Connection, a Run, a Session and a page; any later call on a closed one, on a Locator built on a closed page or on a page of a closed Session fails with `closed`, with no round trip, and closing what was closed with its Run or its Session sends nothing.
- A Connection belongs to the realm that opened it: passing it, or a Run, to a `worker_threads` worker throws a `DataCloneError`; a worker connects on its own (ADR 0081).
- Nothing is read, patched or logged at import, and the SDK writes nothing to stdout or stderr: the engine's stderr goes to `logger`, at warning, when one is given, and so do the engine's log events, at their level, `trace` at `debug`. The engine sends them from the logger's `level` when it is one of its names (`off`, `error`, `warn`, `info`, `debug`, `trace`), else from `info`, and none without a `logger`; the SDK reads it at `connect()` and again when a Run starts, and tells the engine only when it changed (ADR 0078).

Its tests: `src/*.test.ts` on the Scripted engine over the in-memory transport (Quick), `tests/` on an Attached engine of the executable of the same commit (Pre-merge), both running the tracer program in `tests/programs/` (ADR 0069). Its hand-written members are listed in [`../manifest.toml`](../manifest.toml).
