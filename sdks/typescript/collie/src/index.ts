// Where the main types live (ADR 0117): this package, `@goodfetch/collie`,
// holds `Collie`; the Collie client package it re-exports,
// `@goodfetch/collie-client`, holds `Run`, `Session`, `Page` and
// `Locator`, with the results and errors; `@goodfetch/foxhound` holds
// `Foxhound`, `Crawl`, `Label`, `Dataset` and `CrawlContext`.
//
// `@goodfetch/collie` (ADRs 0090 and 0095): Collie's TypeScript SDK and its
// `goodfetch-collie` bin. It re-exports the Collie client package, whose
// values (Run, errors) a Foxhound program gets too, and adds the Connection,
// which finds Collie's executable and reads only
// `GOODFETCH_COLLIE_ENGINE_PATH`. Importing it reads, patches and logs
// nothing (ADR 0078).
export * from "@goodfetch/collie-client";
export { Collie } from "./collie.ts";
