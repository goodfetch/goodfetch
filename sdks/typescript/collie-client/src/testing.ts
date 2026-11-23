// `@goodfetch/collie-client/testing`: the testing kit's shared part (ADRs
// 0069, 0095 and 0105), which each Product's kit builds on and re-exports
// `TestEngineOptions` of. Importing it reads, patches and logs nothing (ADR
// 0078).
export type { TestEngine, TestEngineOptions } from "./test-engine.ts";
