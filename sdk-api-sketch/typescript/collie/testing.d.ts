// Collie's testing kit, `@goodfetch/collie/testing` (ADRs 0069, 0090, 0105): the shared kit of
// `@goodfetch/collie-client/testing`, and a test engine whose connection is `Collie`.

export * from "@goodfetch/collie-client/testing";

import type { TestEngine as SharedTestEngine, TestEngineOptions } from "@goodfetch/collie-client/testing";
import type { Collie } from "./index.js";

/** Collie's test engine: an engine of Collie's executable set up for one test. */
export declare class TestEngine implements SharedTestEngine<Collie> {
  private constructor();
  static start(options?: TestEngineOptions): Promise<TestEngine>;
  readonly connection: Collie;
  reconcile(runId: string, key: string, answer: "happened" | "did_not_happen"): Promise<void>;
  close(): Promise<void>;
  [Symbol.asyncDispose](): Promise<void>;
}
