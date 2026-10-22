// Seam B (spec #326): a failed Playwright test's trace is kept beside the
// JUnit files, which every job uploads (ADR 0068), and a passing test's is
// not. Playwright runs the repository's own `playwright.config.ts` on a
// spec file of test support's `test`, through `repository-playwright.ts`,
// so the run writes nothing into the suite's own output.
import { existsSync, readdirSync } from "node:fs";
import { dirname, join, sep } from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect } from "vitest";
import { test } from "@goodfetch/test-support";
import { runRepositoryPlaywright } from "./repository-playwright.ts";

const FIXTURES = pathToFileURL(join(import.meta.dirname, "..", "src", "playwright.ts")).href;

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
    const { project, code } = await runRepositoryPlaywright(processes, {
      folder: "trace-test",
      spec: { name: "trace.spec.ts", text: SPEC },
    });

    const junit = join(project, "target", "junit", "playwright.xml");
    expect(code).toBe(1);
    expect(existsSync(junit)).toBe(true);
    expect(traces(dirname(junit))).toEqual([
      expect.stringMatching(/^playwright\/trace-fails-its-assertion-default\/trace\.zip$/),
    ]);
  });
});
