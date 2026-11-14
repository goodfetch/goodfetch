// Where the main types live (ADR 0117): `@goodfetch/collie` holds `Collie`;
// the Collie client package holds `Run`, and will hold `Page`, `Session` and
// `Locator`; `@goodfetch/foxhound` holds `Foxhound`, `Crawl`, `Label`,
// `Dataset` and `CrawlContext`.
//
// `@goodfetch/collie-client/runtime`: the hand-written runtime each
// Product's package builds its Connection type on (ADR 0095). Given what
// tells the Product's package from the other's, it finds and starts the
// Product's Attached engine and opens the common Connection on it, `hello`
// naming the Product. It is for the Product packages, never for programs,
// which hold their Product's Connection type.
export type { ClientConnection } from "./client-connection.ts";
export type { ErrorClasses } from "./errors.ts";
export type { Transport } from "./rpc.ts";
export { holdInRealm } from "./realm.ts";
export {
  locateEngine,
  openAttached,
  openOver,
  type EngineLocation,
  type ProductPackage,
} from "./product-connection.ts";
export {
  COLLIE_ENGINE_PATH_VARIABLE,
  COLLIE_EXECUTABLE,
  PRODUCT_VERSION,
} from "./generated/connection.ts";
