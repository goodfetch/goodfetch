// Seam B (spec #326): `visualConfig` gives the `visual` projects of Vitest
// and Playwright only inside the pinned Playwright image, which
// ci/visual/run.sh names in GOODFETCH_VISUAL_IMAGE, and refuses any other
// image (ADR 0073).
import { readFileSync } from "node:fs";
import { playwright as provider } from "@vitest/browser-playwright";
import { describe, expect, test } from "vitest";
import { TestSupportError } from "./errors.ts";
import { visualConfig } from "./visual-config.ts";

// The digest ci/visual/image.toml pins, read the way ci/visual/run.sh reads it.
const PINNED = /^digest = "(.*)"$/m.exec(
  readFileSync(new URL("../../../ci/visual/image.toml", import.meta.url), "utf8"),
)?.[1];
const OTHER_IMAGE = `sha256:${"2".repeat(64)}`;

function refusal(make: () => unknown): string | undefined {
  try {
    make();
  } catch (error) {
    return error instanceof TestSupportError ? error.code : undefined;
  }
  return undefined;
}

describe("visualConfig", () => {
  test("gives no visual project outside the pinned image", () => {
    expect(visualConfig.playwright({}).projects).toEqual([]);
    expect(visualConfig.vitest({}, provider).projects).toEqual([]);
  });

  test("gives the visual projects inside the pinned image", () => {
    const environment = { GOODFETCH_VISUAL_IMAGE: PINNED };

    expect(visualConfig.playwright(environment).projects.map((project) => project.name)).toEqual([
      "visual",
    ]);
    expect(
      visualConfig.vitest(environment, provider).projects.map((project) => project.test?.name),
    ).toEqual(["visual"]);
  });

  // Firefox and WebKit repeated every run of `just visual-repeatability`
  // (REPEATABILITY), so they join in the Release suite, where ADR 0073
  // screenshots them.
  test("gives Firefox's and WebKit's visual projects in the Release suite", () => {
    const environment = { GOODFETCH_VISUAL_IMAGE: PINNED, GOODFETCH_TEST_SUITE: "release" };

    const projects = visualConfig.playwright(environment).projects;

    expect(projects.map((project) => [project.name, project.use?.browserName])).toEqual([
      ["visual", "chromium"],
      ["visual-firefox", "firefox"],
      ["visual-webkit", "webkit"],
    ]);
  });

  test("gives Playwright's visual project on each browser a measurement names", () => {
    const environment = {
      GOODFETCH_VISUAL_IMAGE: PINNED,
      GOODFETCH_VISUAL_BROWSERS: "webkit,firefox",
    };

    const projects = visualConfig.playwright(environment).projects;

    expect(projects.map((project) => [project.name, project.use?.browserName])).toEqual([
      ["visual-firefox", "firefox"],
      ["visual-webkit", "webkit"],
    ]);
  });

  test.each([
    ["a browser the pinned image does not ship", "safari"],
    ["no browser", ""],
  ])("refuses a measurement of %s", (_case, browsers) => {
    const environment = { GOODFETCH_VISUAL_IMAGE: PINNED, GOODFETCH_VISUAL_BROWSERS: browsers };

    expect(refusal(() => visualConfig.playwright(environment))).toBe("visual_browser_unknown");
  });

  test("refuses an image that is not the pinned one", () => {
    const environment = { GOODFETCH_VISUAL_IMAGE: OTHER_IMAGE };

    expect(refusal(() => visualConfig.playwright(environment))).toBe("visual_image_not_pinned");
    expect(refusal(() => visualConfig.vitest(environment, provider))).toBe(
      "visual_image_not_pinned",
    );
  });
});
