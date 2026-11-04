// Seam B (spec #326): the repeatability measurement of ADR 0073, which runs
// every Playwright visual case of a browser many times in the pinned image
// before that browser may join the `visual` projects. Each test copies the
// repository's `playwright.config.ts` unchanged into a folder of its own
// under target/, beside one visual spec file, and measures there on this
// host's Chromium with GOODFETCH_VISUAL_IMAGE naming the pinned digest, as
// ci/visual/run.sh does. The References it writes stay in that folder and
// are deleted with it.
import { copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect, onTestFinished } from "vitest";
import { test } from "@goodfetch/test-support";
import { measureRepeatability } from "../src/repeatability.ts";
import { runnerProject } from "./runner-project.ts";

const ROOT = join(import.meta.dirname, "..", "..", "..");
const SOURCES = join(import.meta.dirname, "..", "src");
const PINNED = /^digest = "(.*)"$/m.exec(
  readFileSync(join(ROOT, "ci", "visual", "image.toml"), "utf8"),
)?.[1];

/** How many times each test runs every case. */
const RUNS = 3;

/**
 * A visual spec file with one case. With `moving` set, the case's square
 * moves by one pixel each run (Playwright's repeat index), as a screen whose
 * pixels change between runs does; its first run matches the Reference the
 * measurement writes.
 */
function spec(options: { readonly moving: boolean }): string {
  return `import { test } from ${JSON.stringify(pathToFileURL(join(SOURCES, "playwright.ts")).href)};
import { screenshot } from ${JSON.stringify(pathToFileURL(join(SOURCES, "screenshot.ts")).href)};

test("square", async ({ page }) => {
  const left = ${options.moving ? "test.info().repeatEachIndex" : "0"};
  await page.clock.setFixedTime(new Date("2026-01-05T09:00:00Z"));
  await page.setContent(\`<!doctype html>
<html lang="en">
  <body style="margin: 0; background: #ffffff">
    <div style="position: absolute; left: \${left}px; top: 16px; width: 40px; height: 40px; background: #0b6e4f"></div>
  </body>
</html>\`);

  await screenshot(page, "square.png");
});
`;
}

/** A folder of its own with the repository's config and `files`. */
function project(files: Readonly<Record<string, string>>): string {
  const folder = runnerProject("repeatability");
  onTestFinished(() => rmSync(folder, { recursive: true, force: true }));
  copyFileSync(join(ROOT, "playwright.config.ts"), join(folder, "playwright.config.ts"));
  // The config reads the Feature inventory beside itself (ADR 0072).
  mkdirSync(join(folder, "features"));
  copyFileSync(
    join(ROOT, "features", "inventory.json"),
    join(folder, "features", "inventory.json"),
  );
  for (const [name, content] of Object.entries(files)) {
    writeFileSync(join(folder, name), content);
  }
  return folder;
}

function environment(): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = { ...process.env, GOODFETCH_VISUAL_IMAGE: PINNED };
  delete env["PLAYWRIGHT_JUNIT_OUTPUT_FILE"];
  return env;
}

describe("measureRepeatability", () => {
  test("finds that a case rendering the same pixels every run repeats", async ({ processes }) => {
    const root = project({ "still.visual.spec.ts": spec({ moving: false }) });

    const verdicts = await measureRepeatability({
      root,
      browsers: ["chromium"],
      runs: RUNS,
      environment: environment(),
      processes,
    });

    expect(verdicts).toEqual([
      {
        browser: "chromium",
        repeats: true,
        cases: [{ file: "still.visual.spec.ts", title: "square", runs: RUNS, repeated: RUNS }],
      },
    ]);
  });

  test("finds that a case whose pixels change between runs does not repeat", async ({
    processes,
  }) => {
    const root = project({ "moving.visual.spec.ts": spec({ moving: true }) });

    const verdicts = await measureRepeatability({
      root,
      browsers: ["chromium"],
      runs: RUNS,
      environment: environment(),
      processes,
    });

    expect(verdicts).toEqual([
      {
        browser: "chromium",
        repeats: false,
        cases: [{ file: "moving.visual.spec.ts", title: "square", runs: RUNS, repeated: 1 }],
      },
    ]);
  });

  test("finds that a browser with no visual case does not repeat", async ({ processes }) => {
    const root = project({});

    const verdicts = await measureRepeatability({
      root,
      browsers: ["chromium"],
      runs: RUNS,
      environment: environment(),
      processes,
    });

    expect(verdicts).toEqual([{ browser: "chromium", repeats: false, cases: [] }]);
  });
});
