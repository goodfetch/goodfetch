// A Vitest project of Console-like tests on the in-memory adapter of the
// protocol, run by tests/protocol-adapter.test.ts one test at a time: each
// but one gives the adapter something the engine could never send or take,
// and must fail; the last two run together, in order, to show nothing of an
// adapter reaches its Client once its test ended. It prints one `ran: <test> <state> <codes>` line per test,
// `<codes>` being the `code` of each error that failed it, and writes its
// caches under GOODFETCH_FIXTURE_OUTPUT, never beside itself.
import { join } from "node:path";
import { defineConfig, type ViteUserConfig } from "vitest/config";
import type { Reporter, TestCase } from "vitest/node";

const output = process.env["GOODFETCH_FIXTURE_OUTPUT"];
if (output === undefined) {
  throw new Error("GOODFETCH_FIXTURE_OUTPUT names the folder this fixture writes in");
}

/** The `code` of an error Vitest serialized, when it has one. */
function codeOf(error: object): string {
  return "code" in error && typeof error.code === "string" ? error.code : "no-code";
}

const outcomes: Reporter = {
  onTestCaseResult(testCase: TestCase) {
    const result = testCase.result();
    if (result.state === "skipped") {
      return;
    }
    const codes = (result.errors ?? []).map(codeOf).join(",");
    process.stdout.write(`ran: ${testCase.name} ${result.state} ${codes}\n`);
  },
};

const config: ViteUserConfig = defineConfig({
  cacheDir: join(output, "vite"),
  test: {
    include: ["adapter.test.ts"],
    retry: 0,
    testTimeout: 10_000,
    reporters: [outcomes],
  },
});

export default config;
