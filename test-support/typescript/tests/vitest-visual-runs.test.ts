// Seam B (spec #326): Vitest's `visual` project, as the shared visual config
// gives it inside the pinned image (ADR 0073). Each test copies the
// repository's `vitest.config.ts` unchanged into a folder of its own
// (`runner-project.ts`), beside one visual test file, and runs Vitest's `visual` project
// there with GOODFETCH_VISUAL_IMAGE naming the pinned digest, as
// ci/visual/run.sh does. Its Reference screenshot is written by the run
// itself, into that folder, and deleted with it: no screenshot rendered here
// is ever committed.
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { describe, expect } from "vitest";
import { test, type ProcessRecord } from "@goodfetch/test-support";
import { runnerProject } from "./runner-project.ts";

const ROOT = join(import.meta.dirname, "..", "..", "..");
const VITEST = join(
  dirname(createRequire(import.meta.url).resolve("vitest/package.json")),
  "vitest.mjs",
);
const PINNED = /^digest = "(.*)"$/m.exec(
  readFileSync(join(ROOT, "ci", "visual", "image.toml"), "utf8"),
)?.[1];

/** Where the run keeps the test file's Reference screenshot. */
const REFERENCE = join("__screenshots__", "page.visual.test.ts", "page-chromium.png");

interface Page {
  /** Which moment of the caret and the animation the screenshot catches. */
  readonly moment: "first" | "second";
  /** Whether one pixel of the page is black. */
  readonly pixel: "unchanged" | "changed";
}

// A page with a caret (a focused text field) and an infinite animation that
// ignores reduced motion, caught at one of two moments, as two runs of a
// blinking caret and a running animation catch them: `first` puts the caret
// before the text and the animation at its start, `second` the caret after
// the text and the animation halfway.
function testFile({ moment, pixel }: Page): string {
  const second = moment === "second";
  const dot =
    pixel === "changed"
      ? '<div style="position: absolute; left: 600px; top: 600px; width: 1px; height: 1px; background: #000000"></div>'
      : "";
  return `import { expect, test } from "vitest";
import { page } from "vitest/browser";

test("caret and animation", async () => {
  document.body.style.margin = "0";
  document.body.style.background = "#ffffff";
  document.body.innerHTML = \`
    <style>
      @keyframes slide { from { transform: translateX(0); } to { transform: translateX(400px); } }
      .moving { width: 40px; height: 40px; background: #0b6e4f; animation: slide 100s linear infinite ${second ? "-50s" : "0s"}; }
    </style>
    <main data-testid="page" style="position: relative; width: 800px; height: 700px">
      <input aria-label="Name" value="Caret" style="margin: 16px; font: 16px sans-serif" />
      <div class="moving"></div>
      ${dot}
    </main>\`;
  const field = document.querySelector("input");
  field?.focus();
  field?.setSelectionRange(${second ? 5 : 0}, ${second ? 5 : 0});

  await expect.element(page.getByTestId("page")).toMatchScreenshot("page");
});
`;
}

/** A folder of its own with the repository's config and the test file. */
function project(): string {
  const folder = runnerProject("vitest-visual-runs");
  copyFileSync(join(ROOT, "vitest.config.ts"), join(folder, "vitest.config.ts"));
  // The config reads the Feature inventory and the Journey catalog beside
  // itself, for its tags (ADRs 0063 and 0072).
  mkdirSync(join(folder, "features"));
  copyFileSync(
    join(ROOT, "features", "inventory.json"),
    join(folder, "features", "inventory.json"),
  );
  mkdirSync(join(folder, "journeys"));
  for (const name of readdirSync(join(ROOT, "journeys")).filter((n) => /^J-\d+\.toml$/.test(n))) {
    copyFileSync(join(ROOT, "journeys", name), join(folder, "journeys", name));
  }
  return folder;
}

/** Runs the visual project on `page` in `folder`, returning its exit code. */
async function visualRun(
  processes: ProcessRecord,
  folder: string,
  page: Page,
  update = false,
): Promise<number | null> {
  writeFileSync(join(folder, "page.visual.test.ts"), testFile(page));
  // Outside CI, where Vitest itself would write a missing reference, and
  // unmeasured: under `just coverage` the copied config would measure the
  // nested run too.
  const env: NodeJS.ProcessEnv = { ...process.env, GOODFETCH_VISUAL_IMAGE: PINNED };
  delete env["CI"];
  delete env["GOODFETCH_COVERAGE"];
  const args = [VITEST, "run", "--config", "vitest.config.ts", "--project", "visual"];
  if (update) {
    args.push("--update");
  }
  const run = processes.spawn(process.execPath, args, { env, cwd: folder });
  run.child.stdout.resume();
  run.child.stderr.resume();
  return (await run.exit).code;
}

describe("Vitest's visual project", () => {
  test("screenshots a page with a blinking caret and an animation identically twice", async ({
    processes,
  }) => {
    const folder = project();
    expect(await visualRun(processes, folder, { moment: "first", pixel: "unchanged" }, true)).toBe(
      0,
    );
    // The run found its test: the root config passes a run with none.
    expect(existsSync(join(folder, REFERENCE))).toBe(true);

    const exit = await visualRun(processes, folder, { moment: "second", pixel: "unchanged" });

    expect(exit).toBe(0);
  });

  test("fails a page one pixel away from its Reference screenshot", async ({ processes }) => {
    const folder = project();
    expect(await visualRun(processes, folder, { moment: "first", pixel: "unchanged" }, true)).toBe(
      0,
    );

    const exit = await visualRun(processes, folder, { moment: "first", pixel: "changed" });

    expect(exit).toBe(1);
  });

  test("fails a missing Reference screenshot and writes none", async ({ processes }) => {
    const folder = project();

    const exit = await visualRun(processes, folder, { moment: "first", pixel: "unchanged" });

    expect(exit).toBe(1);
    expect(existsSync(join(folder, REFERENCE))).toBe(false);
  });
});
