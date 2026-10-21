// Seam C (spec #326): the `conformance` documents the Playwright reporter
// writes for the Console conformance suite (ADR 0068). Each case runs
// Playwright on the fixture configuration of `tests/fixtures/conformance/`,
// whose projects are two targets, Collie's Daemon and Foxhound's, serving
// the stand-in Console, and whose case list declares a capability whose test
// passes, one whose test fails and one no test runs. The reporter writes
// each target's document into a folder of the case's own, which
// `goodfetch-conformance read` loads as every reader does, through
// `goodfetch-protocol`'s reader, which recomputes each outcome.
import { readdirSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { describe, expect } from "vitest";
import {
  test,
  toolPath,
  type ConformanceResultDocument,
  type ProcessRecord,
  type TestEnvDescription,
} from "@goodfetch/test-support";

const CLI = createRequire(import.meta.url).resolve("@playwright/test/cli");
const FIXTURE = join(import.meta.dirname, "fixtures", "conformance");

/** What one Playwright run of the fixture gave. */
interface Ran {
  /** Playwright's exit code. */
  readonly code: number | null;
  /** What Playwright wrote on stdout and stderr, where it reports a reporter's errors. */
  readonly output: string;
  /** The folder the reporter wrote into. */
  readonly documents: string;
}

/** Runs Playwright with `args` on the fixture configuration, writing into the test's folder. */
async function run(
  processes: ProcessRecord,
  testEnv: TestEnvDescription,
  args: readonly string[],
): Promise<Ran> {
  const documents = join(testEnv.folder, "result-documents");
  const playwrightEnv: NodeJS.ProcessEnv = {
    ...process.env,
    GOODFETCH_TEST_RESULT_DOCUMENTS: documents,
  };
  delete playwrightEnv["PLAYWRIGHT_JUNIT_OUTPUT_FILE"];
  const playwright = processes.spawn(
    process.execPath,
    [CLI, "test", "--config", join(FIXTURE, "playwright.config.ts"), ...args],
    { env: playwrightEnv, cwd: FIXTURE },
  );
  playwright.child.stdin.end();
  let output = "";
  for (const stream of [playwright.child.stdout, playwright.child.stderr]) {
    stream.setEncoding("utf8");
    stream.on("data", (chunk: string) => {
      output += chunk;
    });
  }
  const { code } = await playwright.exit;
  return { code, output, documents };
}

/** The document of `target` in `documents`. */
function documentOf(documents: string, target: string): ConformanceResultDocument {
  return JSON.parse(readFileSync(join(documents, `${target}.json`), "utf8"));
}

/** `goodfetch-conformance read`'s exit code on the document of `target` in `documents`. */
async function read(
  processes: ProcessRecord,
  testEnv: TestEnvDescription,
  documents: string,
  target: string,
): Promise<number | null> {
  const reader = processes.spawn(
    toolPath(testEnv.tools, "conformance"),
    ["read", join(documents, `${target}.json`)],
    { env: testEnv.env },
  );
  reader.child.stdin.end();
  reader.child.stdout.resume();
  reader.child.stderr.resume();
  return (await reader.exit).code;
}

describe("the Console conformance reporter", () => {
  test("proves a passing capability, fails a failing one and leaves one no test reaches unproved", async ({
    processes,
    testEnv,
  }) => {
    const ran = await run(processes, testEnv, ["conformance.spec.ts", "--project", "collie"]);
    const document = documentOf(ran.documents, "collie-daemon");

    expect(document.capabilities.map(({ name, outcome }) => [name, outcome])).toEqual([
      ["passing", "proved"],
      ["failing", "failed"],
      ["untested", "unproved"],
    ]);
    expect(await read(processes, testEnv, ran.documents, "collie-daemon")).toBe(0);
    expect(ran.code).toBe(1);
  });

  test("writes one document per target, each named after its target", async ({
    processes,
    testEnv,
  }) => {
    const ran = await run(processes, testEnv, ["conformance.spec.ts"]);

    expect(readdirSync(ran.documents).toSorted()).toEqual([
      "collie-daemon.json",
      "foxhound-daemon.json",
    ]);
    for (const [target, product] of [
      ["collie-daemon", "collie"],
      ["foxhound-daemon", "foxhound"],
    ] as const) {
      expect(documentOf(ran.documents, target).target).toEqual(
        expect.objectContaining({ name: target, product }),
      );
      expect(await read(processes, testEnv, ran.documents, target)).toBe(0);
    }
  });

  test("fails the run when a test runs a case its capability does not declare", async ({
    processes,
    testEnv,
  }) => {
    const ran = await run(processes, testEnv, ["unknown-case.spec.ts", "--project", "collie"]);

    expect(ran.output).toContain("conformance_case_unknown");
    expect(ran.code).toBe(1);
  });
});
