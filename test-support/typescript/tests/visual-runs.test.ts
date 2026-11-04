// Seam B (spec #326): Playwright's `visual` project, as the shared visual
// config gives it inside the pinned image, with the shared screenshot helper
// (ADR 0073). Each test copies the repository's `playwright.config.ts`
// unchanged into a folder of its own under target/, beside one visual spec
// file, and runs Playwright there with GOODFETCH_VISUAL_IMAGE naming the
// pinned digest, as ci/visual/run.sh does. Its Reference screenshot is
// written by the run itself, into that folder, and deleted with it: no
// screenshot rendered here is ever committed.
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect, onTestFinished } from "vitest";
import { test, type ProcessRecord } from "@goodfetch/test-support";
import { runnerProject } from "./runner-project.ts";

const ROOT = join(import.meta.dirname, "..", "..", "..");
const CLI = createRequire(import.meta.url).resolve("@playwright/test/cli");
const SOURCES = join(import.meta.dirname, "..", "src");
const PINNED = /^digest = "(.*)"$/m.exec(
  readFileSync(join(ROOT, "ci", "visual", "image.toml"), "utf8"),
)?.[1];

/** Where the run keeps the spec file's Reference screenshot. */
const REFERENCE = join("page.visual.spec.ts-snapshots", "page-visual.png");

// A page with a caret (a focused text field) and an infinite animation that
// ignores reduced motion. GOODFETCH_FIXTURE_MOMENT picks the moment the
// screenshot catches them at, so two runs see the caret and the animation in
// different states, as two runs of a blinking caret and a running animation
// do: `first` puts the caret before the text and the animation at its start,
// `second` the caret after the text and the animation halfway. With
// GOODFETCH_FIXTURE_PIXEL set to `changed`, one pixel of the page turns black.
const SPEC = `import { test } from ${JSON.stringify(pathToFileURL(join(SOURCES, "playwright.ts")).href)};
import { screenshot } from ${JSON.stringify(pathToFileURL(join(SOURCES, "screenshot.ts")).href)};

const SECOND = process.env["GOODFETCH_FIXTURE_MOMENT"] === "second";
const PIXEL = process.env["GOODFETCH_FIXTURE_PIXEL"] === "changed"
  ? '<div style="position: absolute; left: 600px; top: 600px; width: 1px; height: 1px; background: #000000"></div>'
  : "";

test("caret and animation", async ({ page }) => {
  await page.clock.setFixedTime(new Date("2026-01-05T09:00:00Z"));
  await page.setContent(\`<!doctype html>
<html lang="en">
  <head>
    <style>
      @keyframes slide { from { transform: translateX(0); } to { transform: translateX(400px); } }
      .moving {
        width: 40px; height: 40px; background: #0b6e4f;
        animation: slide 100s linear infinite \${SECOND ? "-50s" : "0s"};
      }
    </style>
  </head>
  <body style="margin: 0; background: #ffffff">
    <input aria-label="Name" value="Caret" style="margin: 16px; font: 16px sans-serif" />
    <div class="moving"></div>
    \${PIXEL}
  </body>
</html>\`);
  const field = page.getByLabel("Name");
  await field.focus();
  await field.evaluate((input, at) => {
    if (input instanceof HTMLInputElement) {
      input.setSelectionRange(at, at);
    }
  }, SECOND ? 5 : 0);

  await screenshot(page, "page.png");
});
`;

/** A folder of its own with the repository's config and the spec file. */
function project(): string {
  const folder = runnerProject("visual-runs");
  onTestFinished(() => rmSync(folder, { recursive: true, force: true }));
  copyFileSync(join(ROOT, "playwright.config.ts"), join(folder, "playwright.config.ts"));
  // The config reads the Feature inventory beside itself (ADR 0072).
  mkdirSync(join(folder, "features"));
  copyFileSync(
    join(ROOT, "features", "inventory.json"),
    join(folder, "features", "inventory.json"),
  );
  writeFileSync(join(folder, "page.visual.spec.ts"), SPEC);
  return folder;
}

/** Runs the visual project in `folder`, returning its exit code. */
async function visualRun(
  processes: ProcessRecord,
  folder: string,
  options: {
    readonly update?: boolean;
    readonly moment?: "first" | "second";
    readonly pixel?: "changed";
  } = {},
): Promise<number | null> {
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    GOODFETCH_VISUAL_IMAGE: PINNED,
    GOODFETCH_FIXTURE_MOMENT: options.moment ?? "first",
    GOODFETCH_FIXTURE_PIXEL: options.pixel ?? "unchanged",
  };
  delete env["PLAYWRIGHT_JUNIT_OUTPUT_FILE"];
  const args = [CLI, "test", "--config", "playwright.config.ts", "--project", "visual"];
  if (options.update === true) {
    args.push("--update-snapshots=all");
  }
  const run = processes.spawn(process.execPath, args, { env, cwd: folder });
  run.child.stdout.resume();
  run.child.stderr.resume();
  return (await run.exit).code;
}

/** Every file under `folder` whose name ends with `suffix`. */
function filesEndingWith(folder: string, suffix: string): string[] {
  return readdirSync(folder, { recursive: true, encoding: "utf8" }).filter((path) =>
    path.endsWith(suffix),
  );
}

describe("the visual project", () => {
  test("screenshots a page with a blinking caret and an animation identically twice", async ({
    processes,
  }) => {
    const folder = project();
    expect(await visualRun(processes, folder, { update: true, moment: "first" })).toBe(0);

    const exit = await visualRun(processes, folder, { moment: "second" });

    expect(exit).toBe(0);
  });

  test("fails a page one pixel away from its Reference screenshot", async ({ processes }) => {
    const folder = project();
    expect(await visualRun(processes, folder, { update: true })).toBe(0);

    const exit = await visualRun(processes, folder, { pixel: "changed" });

    expect(exit).toBe(1);
    expect(filesEndingWith(join(folder, "target"), "page-diff.png")).toHaveLength(1);
  });

  test("fails a missing Reference screenshot and writes none", async ({ processes }) => {
    const folder = project();

    const exit = await visualRun(processes, folder);

    expect(exit).toBe(1);
    expect(existsSync(join(folder, REFERENCE))).toBe(false);
  });
});
