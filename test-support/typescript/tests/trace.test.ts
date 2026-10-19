// Seam B (spec #326): a failed Playwright test's trace is kept beside the
// JUnit files, which every job uploads (ADR 0068), and a passing test's is
// not. Playwright runs the repository's own `playwright.config.ts`, copied
// unchanged into a folder of its own with a spec file of test support's
// `test`, so the run writes nothing into the suite's own output.
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, sep } from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect, onTestFinished } from "vitest";
import { test } from "@goodfetch/test-support";

const ROOT = join(import.meta.dirname, "..", "..", "..");
const FIXTURES = pathToFileURL(join(import.meta.dirname, "..", "src", "playwright.ts")).href;
const CLI = createRequire(import.meta.url).resolve("@playwright/test/cli");

const SPEC = `import { expect, test } from ${JSON.stringify(FIXTURES)};

test("fails its assertion", async ({ page }) => {
  await page.goto("about:blank");
  expect(await page.title()).toBe("a title about:blank never has");
});

test("passes its assertion", async ({ page }) => {
  await page.goto("about:blank");
  expect(await page.title()).toBe("");
});
`;

/** Every `trace.zip` under `folder`, as paths relative to it with `/` separators. */
function traces(folder: string): string[] {
  return readdirSync(folder, { recursive: true, encoding: "utf8" })
    .filter((path) => path.endsWith("trace.zip"))
    .map((path) => path.split(sep).join("/"));
}

describe("the Playwright configuration", () => {
  test("keeps a failed test's trace beside the JUnit file, and a passing test's none", async ({
    processes,
  }) => {
    // Inside the repository, so the copied configuration finds `@playwright/test`.
    mkdirSync(join(ROOT, "target"), { recursive: true });
    const project = mkdtempSync(join(ROOT, "target", "trace-test-"));
    onTestFinished(() => rmSync(project, { recursive: true, force: true }));
    copyFileSync(join(ROOT, "playwright.config.ts"), join(project, "playwright.config.ts"));
    writeFileSync(join(project, "trace.spec.ts"), SPEC);
    const env: NodeJS.ProcessEnv = { ...process.env, GOODFETCH_TEST_SUITE: "pre-merge" };
    delete env["PLAYWRIGHT_JUNIT_OUTPUT_FILE"];

    const run = processes.spawn(
      process.execPath,
      [CLI, "test", "--config", "playwright.config.ts"],
      {
        env,
        cwd: project,
      },
    );

    const exit = await run.exit;
    const junit = join(project, "target", "junit", "playwright.xml");
    expect(exit.code).toBe(1);
    expect(existsSync(junit)).toBe(true);
    expect(traces(dirname(junit))).toEqual([
      expect.stringMatching(/^playwright\/trace-fails-its-assertion-default\/trace\.zip$/),
    ]);
  });
});
