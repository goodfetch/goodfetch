// The smoke-script runner's fixture run (ADR 0069): the scripts of
// `scripts/` beside this file, run by `smoke.test.ts` on the set
// `GOODFETCH_RUNTIME_SET` names, write their case log to the file
// `GOODFETCH_CASE_LOG` names. Their cases are the case-log fixture's.
import { join } from "node:path";
import { CaseLogReporter, clientCaseTags, readClientCases } from "@goodfetch/test-support/case-log";
import { defineConfig, type ViteUserConfig } from "vitest/config";

const caseList = join(import.meta.dirname, "..", "case-log", "cases.toml");
const out = process.env["GOODFETCH_CASE_LOG"];
if (out === undefined) {
  throw new Error("GOODFETCH_CASE_LOG names no file for the case log");
}

const config: ViteUserConfig = defineConfig({
  test: {
    root: import.meta.dirname,
    include: ["smoke.test.ts"],
    retry: 0,
    tags: clientCaseTags(readClientCases(caseList)),
    strictTags: true,
    reporters: ["default", new CaseLogReporter({ out, caseList })],
  },
});

export default config;
