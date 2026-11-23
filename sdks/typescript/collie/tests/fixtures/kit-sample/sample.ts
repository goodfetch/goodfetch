// The testing kit's sample test (ADR 0105), as a program's author writes
// it: it opens a `TestEngine`, runs the code under test on the Product's own
// connection and checks the Run with `node:assert`, so it needs no runner
// plugin. Each runner's file beside it registers it unchanged:
// `sample.vitest.ts` under Vitest and `sample.node-test.ts` under
// `node:test`. The engine comes from the process's environment, as a test
// run sets it. `test-engine.test.ts` runs them; Python's
// `tests/programs/kit_sample_pytest.py` and Rust's
// `tests/it/sdk/test_engine.rs` are its twins.
import { strict as assert } from "node:assert";
import type { Collie } from "@goodfetch/collie";
import { TestEngine } from "@goodfetch/collie/testing";

/** The sample test: a Run the code under test starts on the test engine finishes. */
export async function aRunFinishesOnTheTestEngine(): Promise<void> {
  await using engine = await TestEngine.start();
  const connection: Collie = engine.connection;

  const report = await (await connection.start()).finish();

  assert.equal(report.lifecycle, "finished");
}
