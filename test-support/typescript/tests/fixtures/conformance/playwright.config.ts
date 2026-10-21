// The Console conformance reporter's fixture run (ADR 0068): the spec files
// beside this file, against two targets, each a project naming its target
// in its `metadata`, under another name than its own: Collie's Daemon and
// Foxhound's, each with a Proving ground. The reporter
// reads `cases.toml` beside this file and writes its documents into the
// folder `GOODFETCH_TEST_RESULT_DOCUMENTS` names; Playwright's own output
// goes beside it.
import { join } from "node:path";
import type { ServedOptions } from "@goodfetch/test-support/playwright";
import { defineConfig, type PlaywrightTestConfig } from "@playwright/test";

const documents = process.env["GOODFETCH_TEST_RESULT_DOCUMENTS"];
if (documents === undefined) {
  throw new Error("GOODFETCH_TEST_RESULT_DOCUMENTS names no folder for the result documents");
}

const config: PlaywrightTestConfig<ServedOptions> = defineConfig<ServedOptions>({
  testDir: import.meta.dirname,
  testMatch: "*.spec.ts",
  outputDir: join(documents, "..", "playwright"),
  retries: 0,
  workers: 1,
  reporter: [
    [
      "@goodfetch/test-support/result-document-reporter",
      { caseList: join(import.meta.dirname, "cases.toml") },
    ],
  ],
  projects: (["collie", "foxhound"] as const).map((product) => ({
    name: product,
    metadata: { conformanceTarget: `${product}-daemon` },
    use: { product, provingGroundSeed: 378 },
  })),
});

export default config;
