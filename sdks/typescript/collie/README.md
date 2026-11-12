# @goodfetch/collie

Collie's TypeScript SDK package and its `goodfetch-collie` bin (ADRs 0090 and 0095). It re-exports [`@goodfetch/collie-client`](../collie-client/README.md) and adds `Collie`, the Connection.

```ts
import { Collie } from "@goodfetch/collie";

const collie = await Collie.connect();
const run = await collie.start();
const report = await run.finish();
await collie.close();
```

- `Collie.connect({ env, logger, signal, maxPendingCalls, maxQueuedWriteBytes })` starts Collie's executable as an Attached engine and finishes `hello` before it returns: a missing executable fails with `engine_not_found`, a refused Product with `not_in_product` and a refused protocol version with `protocol_version_unsupported`, each with a `fix`. It reads `GOODFETCH_COLLIE_ENGINE_PATH` from `env` (the process's environment when left out), else the platform package `@goodfetch/collie-<platform>-<arch>`, and gives the engine exactly `env` as its environment; Foxhound's variable is never read. An aborted `signal` fails it with the signal's reason once the engine it started has stopped. Past `maxPendingCalls` calls waiting for their answer (10,000) or `maxQueuedWriteBytes` bytes waiting to be written (64 MiB), a new call fails at once with `connection_overloaded`.
- `collie.start({ runId, tags })` starts a Run; `run.finish()` answers its Run report; `run.close()` stops a Run still running. `close()` is idempotent on both, and any later call fails with `closed`.
- A Connection belongs to the realm that opened it: passing it, or a Run, to a `worker_threads` worker throws a `DataCloneError`; a worker connects on its own (ADR 0081).
- Nothing is read, patched or logged at import, and the SDK writes nothing to stdout or stderr: the engine's stderr goes to `logger`, at warning, when one is given (ADR 0078).

Its tests: `src/*.test.ts` on the Scripted engine over the in-memory transport (Quick), `tests/` on an Attached engine of the executable of the same commit (Pre-merge), both running the tracer program in `tests/programs/` (ADR 0069). Its hand-written members are listed in [`../manifest.toml`](../manifest.toml).
