// Where the main types live (ADR 0117): `@goodfetch/collie` holds `Collie`;
// the Collie client package holds `Run`, and will hold `Page`, `Session` and
// `Locator`; `@goodfetch/foxhound` holds `Foxhound`, `Crawl`, `Label`,
// `Dataset` and `CrawlContext`.
//
// `@goodfetch/collie-client/runtime`: the hand-written runtime each
// Product's package builds its Connection type on (ADR 0095): starting an
// Attached engine, the transport, `hello` and the common Connection. It is
// for the Product packages, never for programs, which hold their Product's
// Connection type.
export { AttachedEngine, type AttachedEngineStart, type EngineExit } from "./attached-engine.ts";
export { ClientConnection, type ClientConnectionOptions } from "./client-connection.ts";
export type { ErrorClasses } from "./errors.ts";
export type { Transport } from "./rpc.ts";
export { holdInRealm } from "./realm.ts";
export {
  COLLIE_ENGINE_PATH_VARIABLE,
  COLLIE_EXECUTABLE,
  PRODUCT_VERSION,
} from "./generated/connection.ts";
