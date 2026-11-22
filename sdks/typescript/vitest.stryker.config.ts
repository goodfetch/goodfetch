// The workspace's Vitest configuration as StrykerJS's full run reads it
// (`stryker.config.mjs`), with four changes:
//
// - It runs the SDK's tests alone: each project inherits this `dir`, which
//   StrykerJS's own `vitest.dir` does not reach (StrykerJS 10.1.1).
// - It leaves out the Client conformance case `nothing-at-import-or-connect`.
//   That case asserts that importing the SDK reads no variable and adds no
//   global, and StrykerJS's instrumented code does both by design to switch
//   its mutants, so the case fails every mutant's run and fails the initial
//   test run. `just test` keeps running the case.
// - It writes no JUnit file and no case log: those are `just test`'s, and
//   every mutant's run would overwrite them.
// - It skips the tests on the test quarantine list
//   (`vitest.stryker.setup.ts`).
import { join } from "node:path";
import type { ViteUserConfig } from "vitest/config";
import workspace from "../../vitest.config.ts";

const config: ViteUserConfig = {
  ...workspace,
  test: {
    ...workspace.test,
    dir: import.meta.dirname,
    setupFiles: [join(import.meta.dirname, "vitest.stryker.setup.ts")],
    reporters: ["default"],
    tagsFilter: ["!client_case/nothing-at-import-or-connect"],
  },
};

export default config;
