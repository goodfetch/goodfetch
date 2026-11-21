// The workspace's Vitest configuration as StrykerJS's full run reads it
// (`stryker.config.mjs`), less the Client conformance case
// `nothing-at-import-or-connect`. That case asserts that importing the SDK reads
// no variable and adds no global, and StrykerJS's instrumented code does both
// by design to switch its mutants, so the case fails every mutant's run and
// fails the initial test run. `just test` keeps running the case.
import { mergeConfig, type ViteUserConfig } from "vitest/config";
import workspace from "../../vitest.config.ts";

const config: ViteUserConfig = mergeConfig(workspace, {
  test: { tagsFilter: ["!client_case/nothing-at-import-or-connect"] },
});

export default config;
