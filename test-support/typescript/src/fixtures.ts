// The Vitest fixtures of `@goodfetch/test-support` (ADR 0062), through
// `test.extend`: a test takes `testEnv` or `env` and gets the same per-test
// setup as a Rust test, from the real `goodfetch-test-env`.
//
// - `testEnv`: the description `goodfetch-test-env` prints, validated
//   against the committed schema; its teardown closes that process's stdin.
// - `processes`: the test's process record; every process started through
//   it, `goodfetch-test-env` included, must have ended by teardown.
//
// Options a test overrides with `test.override`: `toolManifest` and
// `testEnvProgram`.
import { test as base, type TestAPI } from "vitest";
import { ProcessRecord } from "./process-record.ts";
import { startTestEnv, type Program, type TestEnvDescription } from "./test-env.ts";

/** What a test takes from test support's `test`. */
export interface TestSupportFixtures {
  /** The test tool manifest's path: `GOODFETCH_TEST_TOOLS`, which `just test` sets. */
  toolManifest: string | undefined;
  /** What runs in place of the manifest's `goodfetch-test-env`, when set. */
  testEnvProgram: Program | undefined;
  /** The test's process record. */
  processes: ProcessRecord;
  /** The test's setup, as `goodfetch-test-env` describes it. */
  testEnv: TestEnvDescription;
}

/** Vitest's `test` with test support's fixtures. */
export const test: TestAPI<TestSupportFixtures> = base.extend<TestSupportFixtures>({
  toolManifest: process.env["GOODFETCH_TEST_TOOLS"],
  testEnvProgram: undefined,
  // oxlint-disable-next-line no-empty-pattern -- Vitest reads a fixture's dependencies from its first parameter, which must be a destructuring pattern.
  processes: async ({}, use) => {
    const processes = new ProcessRecord();
    await use(processes);
    await processes.close();
  },
  testEnv: async ({ task, processes, toolManifest, testEnvProgram }, use) => {
    const started = await startTestEnv({
      processes,
      testName: task.fullName,
      toolManifest,
      program: testEnvProgram,
    });
    await use(started.description);
    await started.close();
  },
});
