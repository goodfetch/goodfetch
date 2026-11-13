# @goodfetch/foxhound

Foxhound's TypeScript SDK package (ADRs 0090 and 0095). It re-exports [`@goodfetch/collie-client`](../collie-client/README.md), the only Collie package it depends on, and adds `Foxhound`, the Connection. Area 11 fills the rest: `Crawl`, `Label`, `Dataset` and `CrawlContext`.

```ts
import { Foxhound } from "@goodfetch/foxhound";

const foxhound = await Foxhound.connect();
const run = await foxhound.start();
const report = await run.finish();
await foxhound.close();
```

- `Foxhound.connect({ env, logger, signal, maxPendingCalls, maxQueuedWriteBytes })` starts Foxhound's executable as an Attached engine and finishes `hello`, naming Foxhound, before it returns. A Collie engine refuses it there with `not_in_product` (`NotInProductError`), before any other call: its `details` name the engine's Product and version (`product`, `product_version`), what the Client needs (`needs`) and the install command (`install`), and its `fix` says what to do. A missing executable fails with `engine_not_found` and a refused protocol version with `protocol_version_unsupported`, each with a `fix`. It reads `GOODFETCH_FOXHOUND_ENGINE_PATH` from `env` (the process's environment when left out), else the platform package `@goodfetch/foxhound-<platform>-<arch>`, and gives the engine exactly `env` as its environment; Collie's variable is never read.
- A `Foxhound` carries every member of a `Collie` under the same name (`engine`, `start`, `close`), and both implement the client package's `Connection`, so a helper typed for `Connection` takes either. Its Run, results and error classes are the client package's own: a Collie error class catches the error from either Connection.
- A Connection belongs to the realm that opened it, many may be open in one process, and nothing is read, patched or logged at import: no signal handler is installed (ADRs 0078 and 0081).
- `@goodfetch/foxhound/testing` holds `connectOver(transport)`, which opens the same Connection over a transport a test holds, such as the Scripted engine's in-memory player (ADR 0069).
- `./generated` is the layer `xtask codegen` writes from Foxhound's part of the catalog, with `FOXHOUND_ENGINE_PATH_VARIABLE` and `FOXHOUND_EXECUTABLE`; never edited by hand.

Its tests: `src/*.test.ts` on the Scripted engine over the in-memory transport (Quick), `tests/` on the Scripted engine's process, started as `Foxhound.connect()` starts an Attached engine (Pre-merge), both running the tracer program in `tests/programs/`, which is typed for `Connection`. Its hand-written members are listed in [`../manifest.toml`](../manifest.toml).
