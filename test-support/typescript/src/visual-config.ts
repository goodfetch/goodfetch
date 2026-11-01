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

/** The browsers the pinned image ships. */
export const VISUAL_BROWSERS = ["chromium", "firefox", "webkit"] as const;

/** A browser the pinned image ships. */
export type VisualBrowser = (typeof VISUAL_BROWSERS)[number];

/** Playwright's `visual` project on `browser`: `visual` on Chromium, `visual-<browser>` on another. */
export function visualProject(browser: VisualBrowser): string {
  return browser === "chromium" ? "visual" : `visual-${browser}`;
}

/** What `just visual-repeatability` found on a browser in the pinned image (ADR 0073). */
export type Repeatability =
  /** Every run of every visual case gave the same pixels: the browser is in the `visual` projects. */
  | { readonly verdict: "repeats"; readonly runs: string }
  /** A run gave other pixels: the browser stays out until 20 runs repeat, `tracking` says where that is followed. */
  | { readonly verdict: "differs"; readonly runs: string; readonly tracking: string };

/** The measurements every browser's verdict rests on. */
const MEASURED =
  "2026-10-20, `just visual-repeatability` on 7504f560: every Playwright visual case (1, the pinned image's fixture page) " +
  "repeated its pixels in 20 of 20 runs, in the pinned image on an Apple M3 Max through Docker Desktop's Rosetta for Linux. " +
  "2026-10-22, `just visual-repeatability chromium,firefox,webkit` on e966a3b6: both Playwright visual cases, the fixture page " +
  "and a page served under the production CSP (`tests/csp-page.visual.spec.ts`), repeated their pixels in 20 of 20 runs each, " +
  "in the pinned image under rootless Podman on an AMD EPYC-Milan (Linux x86-64, no emulation)";

/**
 * Each browser's screenshot repeatability in the pinned image, with the runs
 * it rests on (ADR 0073). A browser is in the `visual` projects only when its
 * verdict is `repeats`: Chromium in every suite, another browser in the
 * Release suite, where ADR 0073 screenshots Firefox and WebKit. Rerun the
 * measurement with `just visual-repeatability` and record what it prints.
 */
export const REPEATABILITY: Readonly<Record<VisualBrowser, Repeatability>> = {
  chromium: { verdict: "repeats", runs: `${MEASURED}.` },
  firefox: { verdict: "repeats", runs: `${MEASURED}.` },
  webkit: {
    verdict: "repeats",
    runs:
      `${MEASURED}. On WebKit, Playwright adds an inline <style> before each screenshot, which the production CSP ` +
      "refuses; the shared screenshot helper tolerates exactly that refusal as a known Playwright issue (`known-issues.ts`), " +
      "so the served case passes `failOnErrors` there, as the 2026-10-22 measurement shows.",
  },
};

/**
 * The browsers `list` names, comma-separated, in VISUAL_BROWSERS' order;
 * `visual_browser_unknown` for a name the pinned image does not ship, or none.
 */
export function parseVisualBrowsers(list: string): readonly VisualBrowser[] {
  const names = list.split(",").map((name) => name.trim());
  const browsers = VISUAL_BROWSERS.filter((browser) => names.includes(browser));
  const unknown = names.filter((name) => !VISUAL_BROWSERS.some((browser) => browser === name));
  if (unknown.length > 0 || browsers.length === 0) {
    throw new TestSupportError(
      "visual_browser_unknown",
      `the browsers are ${JSON.stringify(list)}: name one or more of ${VISUAL_BROWSERS.join(", ")}, separated by commas`,
    );
  }
  return browsers;
}

/** Where a project's visual tests are not. */
export interface VisualFiles {
  readonly ignore?: Array<string | RegExp>;
}

/** The shared visual config of both runners' root configs. */
export const visualConfig = {
  playwright(environment: Environment, files: VisualFiles = {}): PlaywrightVisualConfig {
    if (!inPinnedImage(environment)) {
      return { updateSnapshots: "none", projects: [] };
    }
    const projects = visualBrowsers(environment).map((browserName): Project => ({
      name: visualProject(browserName),
      testMatch: PLAYWRIGHT_VISUAL_TESTS,
      ...(files.ignore === undefined ? {} : { testIgnore: files.ignore }),
      use: { browserName, viewport: VISUAL_VIEWPORT, ...FIXED_SETTINGS },
      expect: {
        toHaveScreenshot: {
          maxDiffPixels: 0,
          pathTemplate:
            "{snapshotDir}/{testFileDir}/{testFileName}-snapshots/{arg}{-projectName}{ext}",
        },
      },
    }));
    return { updateSnapshots: "none", projects };
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

/**
 * The browsers Playwright's `visual` projects run on: those
 * GOODFETCH_VISUAL_BROWSERS names (comma-separated), which the repeatability
 * measurement sets, whatever their verdict; otherwise each browser whose
 * screenshots repeat, Chromium in every suite and another browser in the
 * Release suite.
 */
function visualBrowsers(environment: Environment): readonly VisualBrowser[] {
  const measured = environment["GOODFETCH_VISUAL_BROWSERS"];
  if (measured !== undefined) {
    return parseVisualBrowsers(measured);
  }
  const release = environment["GOODFETCH_TEST_SUITE"] === "release";
  return VISUAL_BROWSERS.filter(
    (browser) =>
      REPEATABILITY[browser].verdict === "repeats" && (browser === "chromium" || release),
  );
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
