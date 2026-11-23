// Where the main types live (ADR 0117): `@goodfetch/collie` holds `Collie`;
// the Collie client package this package re-exports,
// `@goodfetch/collie-client`, holds `Run`, `Session`, `Page` and `Locator`,
// with the results and errors; this package, `@goodfetch/foxhound`, holds
// `Foxhound`, `Crawl`, `Label`, `Dataset` and `CrawlContext`.
//
// `@goodfetch/foxhound` (ADRs 0090 and 0095): Foxhound's TypeScript SDK. It
// re-exports the Collie client package, so a Foxhound program's Run, results
// and error classes are the ones a Collie program holds, and adds the
// Connection, which finds Foxhound's executable and reads only
// `GOODFETCH_FOXHOUND_ENGINE_PATH`, the crawl a program declares and starts
// (ADR 0093), and the classes of Foxhound's own Error codes, each under a
// group class of the client package. It depends on no Collie package but the
// client package. Importing it reads, patches and logs nothing (ADR 0078).
export * from "@goodfetch/collie-client";
export * from "./generated/errors.ts";
export {
  Foxhound,
  type FoxhoundConnectOptions,
  type FoxhoundConnectOverOptions,
  type FoxhoundHandlerlessStartOptions,
  type FoxhoundLimits,
  type FoxhoundStartOptions,
  type StartCrawlOptions,
} from "./foxhound.ts";
export {
  Crawl,
  Dataset,
  Label,
  seed,
  type CrawlContext,
  type DatasetOptions,
  type EnqueueLinksOptions,
  type Handler,
  type LabelOptions,
  type PayloadInput,
  type PayloadSchema,
  type PayloadThen,
  type RequestOptions,
} from "./crawl.ts";
export { CrawlRun, type RecordsOptions } from "./crawl-run.ts";
export type {
  CrawlSettings,
  DatasetRecord,
  Enqueued,
  EnqueuedLink,
  EnqueuedLinks,
  FoxhoundSettings,
  LabelSettings,
  RequestSeed,
} from "./generated/index.ts";
