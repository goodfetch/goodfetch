// Where the main types live (ADR 0095): this package, `@goodfetch/collie-client`, holds `Page`,
// `Locator`, `Run`, `Session` and the results and errors; `@goodfetch/collie` holds `Collie`;
// `@goodfetch/foxhound` holds `Foxhound`, `Crawl`, `Label`, `Dataset` and `CrawlContext`. Both
// Products' packages re-export this one.
//
// `@goodfetch/collie-client` (ADR 0095): the Collie client package, with no executable. It holds the
// hand-written runtime, the generated layer of Collie's catalog and every value a Collie connection
// hands out (Runs, Sessions, Pages, Locators, results, errors, settings types). Programs import their
// Product's package, `@goodfetch/collie` or `@goodfetch/foxhound`, which re-exports this one; only a
// library meant to serve both Products imports it, as a peer dependency.
export * from "./agent.js";
export * from "./ai.js";
export * from "./catalog.js";
export * from "./concurrency.js";
export * from "./data.js";
export * from "./effects.js";
export * from "./lending.js";
export * from "./lifecycle.js";
export * from "./models.js";
export * from "./output.js";
export * from "./page.js";
export * from "./plugins.js";
export * from "./politeness.js";
export * from "./recording.js";
export * from "./results.js";
export * from "./run.js";
export * from "./sessions.js";
export * from "./settings.js";
export * from "./time.js";
