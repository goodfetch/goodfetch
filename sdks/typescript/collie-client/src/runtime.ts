// Where the main types live (ADR 0117): `@goodfetch/collie` holds `Collie`;
// the Collie client package holds `Run`, `Page`, `Session` and `Locator`;
// `@goodfetch/foxhound` holds `Foxhound`, `Crawl`, `Label`, `Dataset` and
// `CrawlContext`.
//
// `@goodfetch/collie-client/runtime`: the hand-written runtime each
// Product's package builds its Connection type on (ADR 0095). Given what
// tells the Product's package from the other's, it finds and starts the
// Product's Attached engine and opens the common Connection on it, `hello`
// naming the Product, through which the Product's package reaches its own
// operations, streams and Run type and serves the engine's requests to its
// program's handlers. It is for the Product packages, never for programs,
// which hold their Product's Connection type. Its generated members of a
// deprecated or Experimental entry warn through `markers`' functions.
export type { ClientConnection, Served, Stream } from "./client-connection.ts";
export type { ErrorClasses } from "./errors.ts";
export {
  FileSaving,
  saveFile,
  writeFileWhole,
  type FileWriteOptions,
  type Saved,
} from "./file-saving.ts";
export type { Transport } from "./rpc.ts";
export { holdInRealm } from "./realm.ts";
export { alsoInNamespace } from "./raw-call.ts";
export { warnDeprecated, warnExperimental } from "./markers.ts";
export { ensureRunOpen, pageOf, type RunHost } from "./run.ts";
export { Sequence, type SequenceOptions } from "./sequence.ts";
export { onWire, type SettingsOnWire, type Spelling } from "./settings.ts";
export { idOfLocator } from "./generated/handles.ts";
export { collieTimes, type CollieTimeType } from "./generated/times.ts";
export { withTimes, type TimePath, type Times } from "./times.ts";
export {
  locateEngine,
  openEngine,
  openOver,
  type EngineLocation,
  type ProductPackage,
} from "./product-connection.ts";
export {
  COLLIE_ENGINE_PATH_VARIABLE,
  COLLIE_EXECUTABLE,
  PRODUCT_VERSION,
} from "./generated/connection.ts";
