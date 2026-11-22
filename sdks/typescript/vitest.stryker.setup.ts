// Skips the tests on the test quarantine list in StrykerJS's full run
// (`vitest.stryker.config.ts`), as `just test` does (ADR 0064). StrykerJS
// sets Vitest's own name pattern to choose each mutant's tests, so the
// pattern `xtask quarantine exclude` prints for Vitest, which matches every
// name but the listed ones, comes from `just mutants-sdks` in this variable.
import { beforeEach } from "vitest";

const pattern = process.env["GOODFETCH_STRYKER_NAME_PATTERN"];
if (pattern !== undefined) {
  const runs = new RegExp(pattern);
  beforeEach(({ task, skip }) => {
    skip(!runs.test(task.fullTestName), "on the test quarantine list (quarantine.toml)");
  });
}
