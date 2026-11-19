# @goodfetch/foxhound

Foxhound's TypeScript SDK package (ADRs 0090 and 0095). It re-exports [`@goodfetch/collie-client`](../collie-client/README.md), the only Collie package it depends on, and adds `Foxhound`, the Connection, and a crawl's declaration and Run: `Crawl`, `Label`, `Dataset`, `CrawlContext` and `CrawlRun` (ADRs 0093, 0102 and 0103).

```ts
import { Foxhound } from "@goodfetch/foxhound";

const foxhound = await Foxhound.connect();
const run = await foxhound.start();
const report = await run.finish();
await foxhound.close();
```

```ts
import { Crawl, Foxhound, seed } from "@goodfetch/foxhound";

// Declared at module scope: a Crawl holds no connection and does no I/O.
const crawl = new Crawl();
const listing = crawl.label("listing");
const products = crawl.dataset<{ id: string; name: string }>("products", { key: ["id"] });
crawl.handle(listing, async (context) => {
  const { values } = await context.page.read({ id: { locator: { kind: "css", selector: "[data-id]" }, attribute: "data-id" } });
  await context.push(products, { id: String(values["id"]), name: "…" });
  await context.enqueue("/catalog/2", listing);
});

const foxhound = await Foxhound.connect();
const run = await foxhound.startCrawl(crawl, { seeds: [seed("https://shop.example/catalog/1", listing)] });
const report = await run.finish();
for await (const record of run.records(products)) console.log(record.value);
await foxhound.close();
```

- `Foxhound.connect({ env, logger, signal, maxPendingCalls, maxQueuedWriteBytes, settings })` starts Foxhound's executable as an Attached engine and finishes `hello`, naming Foxhound, before it returns. A Collie engine refuses it there with `not_in_product` (`NotInProductError`), before any other call: its `details` name the engine's Product and version (`product`, `product_version`), what the Client needs (`needs`) and the install command (`install`), and its `fix` says what to do. A missing executable fails with `engine_not_found` and a refused protocol version with `protocol_version_unsupported`, each with a `fix`. It reads `GOODFETCH_FOXHOUND_ENGINE_PATH` from `env` (the process's environment when left out), else the platform package `@goodfetch/foxhound-<platform>-<arch>`, and gives the engine exactly `env` as its environment; Collie's variable is never read. Its `logger` gets what a Collie Connection's does: the engine's stderr lines at warning and the engine's log events at their level, sent from the logger's `level` (ADR 0078).
- `settings` holds the Connection's own settings (ADR 0083), a `FoxhoundSettings`, Collie's keys and Foxhound's nested by camelCase segment, sent in `hello`, beneath each Run's own `settings`: a key left out inherits, `null` clears and a value sets. `timeout.run` is set there only, and a Run's start refuses it in its own `settings` with `setting_not_applicable`; a key the Connection does not take fails `connect()`. `explainSettings(keys?)` on the Connection and on a live Run answers each setting's effective value, layer and origin, as `goodfetch config explain` does.
- A `Foxhound` carries every member of a `Collie` under the same name (`engine`, `start`, `close`), and both implement the client package's `Connection`, so a helper typed for `Connection` takes either. Its Run, results and error classes are the client package's own: a Collie error class catches the error from either Connection. The classes of the Error codes only Foxhound carries (`HandlerFailedError`, `ClientGoneError` and the rest of `./generated`'s `errors.ts`) are exported beside them, each under a group class of the client package.
- `new Crawl()` declares a crawl: `crawl.label(name, { settings })` returns a `Label`, `crawl.handle(label, handler)` attaches its one handler in a second step, so a handler can enqueue to its own Label, and `crawl.dataset<I>(name, { key })` returns a typed `Dataset<I>`. `foxhound.startCrawl(crawl, { seeds, runId, tags, settings })` sends `crawl.start` and returns a `CrawlRun` once started; two Runs started from one `Crawl` share nothing. Its `settings` are a `CrawlSettings`, Collie's Run settings and Foxhound's, and a Label's are a `LabelSettings`, the keys a Label sets over its Run's (`fetch.tier`, `session.page_check` and the others whose catalog scopes name `label`), both nested by camelCase segment and sent in the wire's `snake_case`, so a Run-wide key such as `politeness` on a Label fails to compile (ADRs 0083 and 0093). The engine leases each work item to its Label's handler with a `crawl_request.handle` request, which the handler answers on the event loop with a `CrawlContext`: `page` (a Collie `Page`), `enqueue(url, label, payload?, options?)` and `push(dataset, record)`, both staged with the attempt; a thrown error fails the attempt, with `handler_failed` unless it is a Goodfetch error. `run.records(dataset)` reads the Dataset's committed Records as the engine streams them; `run.close()` stops every attempt still running, whose calls then fail with `closed`. Wiring mistakes fail at once, with nothing sent: a second Label of one name, or a second handler for one Label, raises `label_duplicate`; a Label without a handler raises `label_unhandled` from `startCrawl`; a Label the crawl does not declare, such as another crawl's, raises `label_unknown` from `handle`, `enqueue` and `enqueueLinks`; and a seed whose Label's name the crawl does not declare raises `label_unknown` from `startCrawl`.
- A Connection belongs to the realm that opened it, many may be open in one process, and nothing is read, patched or logged at import: no signal handler is installed (ADRs 0078 and 0081).
- `@goodfetch/foxhound/testing` holds `connectOver(transport)`, which opens the same Connection over a transport a test holds, such as the Scripted engine's in-memory player (ADR 0069).
- `./generated` is the layer `xtask codegen` writes from Foxhound's part of the catalog, with `FOXHOUND_ENGINE_PATH_VARIABLE` and `FOXHOUND_EXECUTABLE`; never edited by hand.

Its tests: `src/*.test.ts` on the Scripted engine over the in-memory transport (Quick), `tests/` on the Scripted engine's process, started as `Foxhound.connect()` starts an Attached engine (Pre-merge), the Run tests running the tracer program in `tests/programs/`, which is typed for `Connection`, and the crawl tests the two-page-types program there; `tests/crawl.test.ts` crawls the Proving ground's `catalog` family on Foxhound's executable with `tests/programs/catalog.ts` and has the oracle judge its Records. Its hand-written members are listed in [`../manifest.toml`](../manifest.toml).
