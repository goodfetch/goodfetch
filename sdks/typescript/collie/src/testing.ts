// Where the main types live (ADR 0117): `@goodfetch/collie` holds `Collie`;
// the Collie client package holds `Run`, and will hold `Page`, `Session` and
// `Locator`; `@goodfetch/foxhound` holds `Foxhound`, `Crawl`, `Label`,
// `Dataset` and `CrawlContext`.
//
// `@goodfetch/collie/testing` (ADRs 0069 and 0105): `TestEngine` starts a
// real Collie engine for a test, with the client package's
// `TestEngineOptions`; `connectOver` opens Collie's Connection over a
// transport the test holds, such as the Scripted engine's in-memory player,
// so a program's tests run its code with no engine process. Importing it
// reads, patches and logs nothing (ADR 0078).
export type { ConnectOverOptions } from "@goodfetch/collie-client";
export type { TestEngineOptions } from "@goodfetch/collie-client/testing";
export { connectOver } from "./collie.ts";
export { TestEngine } from "./test-engine.ts";
