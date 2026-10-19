// The shared visual config (ADR 0073; TESTING.md, Visual regression): the
// `visual` projects of Playwright and Vitest, which exist only inside the
// pinned Playwright image of ci/visual/image.toml (ci/visual/run.sh names it
// in GOODFETCH_VISUAL_IMAGE), compare at zero mismatched pixels at each
// runner's default per-pixel threshold, and write no Reference screenshot in
// a suite. This is the one file that may set a comparison's tolerance:
// `xtask screenshots check` (Quick suite) fails on one set anywhere else.
//
// Each Reference screenshot a run reads is recorded under
// target/visual/read/ for the orphan check (`xtask screenshots orphans`):
// Vitest's here, Playwright's by the shared helper (`./screenshot.ts`).
import { appendFileSync, mkdirSync, readFileSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import type { Project } from "@playwright/test";
import type { playwright } from "@vitest/browser-playwright";
import type { TestProjectInlineConfiguration } from "vitest/config";
import { TestSupportError } from "./errors.ts";

/** Environment variables, as `process.env` holds them. */
export type Environment = Readonly<Record<string, string | undefined>>;

/** The page settings every Reference screenshot is taken under. */
export const FIXED_SETTINGS = {
  locale: "en-US",
  timezoneId: "UTC",
  deviceScaleFactor: 1,
  reducedMotion: "reduce",
} as const;

/** The `visual` projects' viewport, in CSS pixels. */
export const VISUAL_VIEWPORT = { width: 1280, height: 800 } as const;

/** Playwright's visual test files. */
export const PLAYWRIGHT_VISUAL_TESTS = "**/*.visual.spec.ts";

/** Vitest's visual test files. */
export const VITEST_VISUAL_TESTS = "**/*.visual.test.{ts,tsx}";

/** Where each run records the references it read, one repository path a line. */
export const READ_RECORDS = ["target", "visual", "read"] as const;

/** What the root config gives Playwright. */
export interface PlaywrightVisualConfig {
  /** No suite writes a Reference screenshot, even a missing one (Playwright's default would). */
  readonly updateSnapshots: "none";
  /** The `visual` project inside the pinned image; none elsewhere. */
  readonly projects: Project[];
}

/** What the root config gives Vitest. */
export interface VitestVisualConfig {
  /** No suite writes a Reference screenshot; set only inside the pinned image. */
  readonly update?: "none";
  /** The `visual` project inside the pinned image; none elsewhere. */
  readonly projects: TestProjectInlineConfiguration[];
}

/** Where a project's visual tests are not. */
export interface VisualFiles {
  readonly ignore?: Array<string | RegExp>;
}

/** The shared visual config of both runners' root configs. */
export const visualConfig = {
  playwright(environment: Environment, files: VisualFiles = {}): PlaywrightVisualConfig {
    const project: Project = {
      name: "visual",
      testMatch: PLAYWRIGHT_VISUAL_TESTS,
      ...(files.ignore === undefined ? {} : { testIgnore: files.ignore }),
      use: { browserName: "chromium", viewport: VISUAL_VIEWPORT, ...FIXED_SETTINGS },
      expect: {
        toHaveScreenshot: {
          maxDiffPixels: 0,
          pathTemplate:
            "{snapshotDir}/{testFileDir}/{testFileName}-snapshots/{arg}{-projectName}{ext}",
        },
      },
    };
    return { updateSnapshots: "none", projects: inPinnedImage(environment) ? [project] : [] };
  },

  /** `provider` is `@vitest/browser-playwright`'s, which only Vitest's config process loads. */
  vitest(
    environment: Environment,
    provider: typeof playwright,
    files: { readonly exclude?: string[] } = {},
  ): VitestVisualConfig {
    if (!inPinnedImage(environment)) {
      return { projects: [] };
    }
    const project: TestProjectInlineConfiguration = {
      extends: true,
      test: {
        name: "visual",
        include: [VITEST_VISUAL_TESTS],
        ...(files.exclude === undefined ? {} : { exclude: files.exclude }),
        testTimeout: 300_000,
        // A failed comparison's actual and diff images, uploaded by the visual job.
        attachmentsDir: "target/vitest-attachments",
        browser: {
          enabled: true,
          headless: true,
          provider: provider({ contextOptions: FIXED_SETTINGS }),
          instances: [{ browser: "chromium" }],
          viewport: VISUAL_VIEWPORT,
          expect: {
            toMatchScreenshot: {
              comparatorName: "pixelmatch",
              comparatorOptions: { allowedMismatchedPixels: 0 },
              screenshotOptions: { animations: "disabled", caret: "hide" },
              resolveScreenshotPath: (reference) => {
                const path = resolve(
                  reference.root,
                  reference.testFileDirectory,
                  reference.screenshotDirectory,
                  reference.testFileName,
                  `${reference.arg}-${reference.browserName}${reference.ext}`,
                );
                recordReference(reference.root, path, "vitest");
                return path;
              },
            },
          },
        },
      },
    };
    return { update: "none", projects: [project] };
  },
};

/** Records that a run read the Reference screenshot at `path`, for the orphan check. */
export function recordReference(root: string, path: string, runner: "vitest" | "playwright"): void {
  const folder = join(root, ...READ_RECORDS);
  mkdirSync(folder, { recursive: true });
  const line = relative(root, path).split(sep).join("/");
  appendFileSync(join(folder, `${runner}-${process.pid}.txt`), `${line}\n`);
}

/** Whether this process runs in the pinned image; another image is refused. */
function inPinnedImage(environment: Environment): boolean {
  const image = environment["GOODFETCH_VISUAL_IMAGE"];
  if (image === undefined) {
    return false;
  }
  const pinned = pinnedImage();
  if (image !== pinned) {
    throw new TestSupportError(
      "visual_image_not_pinned",
      `GOODFETCH_VISUAL_IMAGE is ${image}, not ci/visual/image.toml's ${pinned}: the visual projects run only in the pinned image, through ci/vm/visual.sh`,
    );
  }
  return true;
}

/** The digest ci/visual/image.toml pins. */
function pinnedImage(): string {
  const file = new URL("../../../ci/visual/image.toml", import.meta.url);
  const digest = /^digest = "(sha256:[0-9a-f]{64})"$/m.exec(readFileSync(file, "utf8"))?.[1];
  if (digest === undefined) {
    throw new TestSupportError(
      "visual_image_not_pinned",
      "ci/visual/image.toml names no sha256 digest",
    );
  }
  return digest;
}
