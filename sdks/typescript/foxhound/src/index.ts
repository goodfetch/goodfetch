// Where the main types live (ADR 0117): `@goodfetch/collie` holds `Collie`;
// the Collie client package this package re-exports,
// `@goodfetch/collie-client`, holds `Run`, `Session`, `Page` and `Locator`,
// with the results and errors; this package, `@goodfetch/foxhound`, holds
// `Foxhound`, and will hold `Crawl`, `Label`, `Dataset` and `CrawlContext`.
//
// `@goodfetch/foxhound` (ADRs 0090 and 0095): Foxhound's TypeScript SDK. It
// re-exports the Collie client package, so a Foxhound program's Run, results
// and error classes are the ones a Collie program holds, and adds the
// Connection, which finds Foxhound's executable and reads only
// `GOODFETCH_FOXHOUND_ENGINE_PATH`. It depends on no Collie package but the
// client package. Importing it reads, patches and logs nothing (ADR 0078).
export * from "@goodfetch/collie-client";
export { Foxhound } from "./foxhound.ts";
