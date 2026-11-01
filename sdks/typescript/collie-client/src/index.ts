// Where the main types live (ADR 0117): `@goodfetch/collie` holds `Collie`;
// this Collie client package, `@goodfetch/collie-client`, which it
// re-exports, holds `Run`, and will hold `Page`, `Session` and `Locator`,
// with the results and errors; `@goodfetch/foxhound` holds `Foxhound`,
// `Crawl`, `Label`, `Dataset` and `CrawlContext`.
//
// `@goodfetch/collie-client` (ADR 0095): the values every Product's
// Connection hands out, a Foxhound program's included, the error classes of
// Collie's Error codes and the Connection both Products' types implement.
// Programs import it through their Product's package; a library serving both
// Products imports it directly. Importing it reads, patches and logs nothing
// (ADR 0078).
export type { Connection } from "./connection.ts";
export type { EngineInfo, StartOptions } from "./client-connection.ts";
export type { Logger } from "./logger.ts";
export { Run } from "./run.ts";
export {
  KeyValueStore,
  type JsonValue,
  type StoredBytes,
  type StoredValue,
} from "./key-value-store.ts";
export * from "./generated/errors.ts";
export type {
  BlockCause,
  CallOptions,
  Completeness,
  Lifecycle,
  Product,
  RunReport,
  StartedAs,
  StopReason,
  StoppedBy,
  Verdict,
} from "./generated/index.ts";
