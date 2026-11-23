// Collie's testing kit (issue 3398, ADR 0105), on the real engine:
// `TestEngine.start()` starts an Attached engine of Collie's executable of
// the same commit in a temporary Goodfetch home it removes on close. Rust's
// `tests/it/sdk/test_engine.rs` and Python's `test_test_engine.py` are its
// twins.
import { stat } from "node:fs/promises";
import { TestEngine } from "@goodfetch/collie/testing";
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** Whether `path` names something on disk. */
async function exists(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

describe("a test engine", () => {
  test(
    "runs a test's code on its connection, and close removes its home",
    { tags: ["attached"] },
    async ({ env }) => {
      const engine = await TestEngine.start({ env });
      const home = engine.connection.engine().home;
      if (home === null) {
        throw new TypeError("an Attached engine names its home");
      }
      const existed = await exists(home);

      const run = await engine.connection.start();
      const report = await run.finish();
      await engine.close();

      expect(report.lifecycle).toBe("finished");
      expect(home).not.toBe(env["GOODFETCH_HOME"]);
      expect([existed, await exists(home)]).toEqual([true, false]);
    },
  );
});
