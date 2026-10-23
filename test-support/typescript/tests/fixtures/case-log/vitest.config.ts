// The case-log reporter's fixture run (ADR 0069): the tests beside this file,
// tagged from `cases.toml` beside it, write their case log to the file
// `GOODFETCH_CASE_LOG` names.
import { join } from "node:path";
import { CaseLogReporter, clientCaseTags, readClientCases } from "@goodfetch/test-support/case-log";
import { defineConfig, type ViteUserConfig } from "vitest/config";

const caseList = join(import.meta.dirname, "cases.toml");
const out = process.env["GOODFETCH_CASE_LOG"];
if (out === undefined) {
  throw new Error("GOODFETCH_CASE_LOG names no file for the case log");
}

const config: ViteUserConfig = defineConfig({
  test: {
    root: import.meta.dirname,
    include: ["conformance.test.ts"],
    retry: 0,
    tags: clientCaseTags(readClientCases(caseList)),
    strictTags: true,
    reporters: ["default", new CaseLogReporter({ out, caseList })],
  },
});

export default config;
