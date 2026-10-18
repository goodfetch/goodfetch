// The shared screenshot helper of Playwright's `visual` project (ADR 0073;
// TESTING.md, Visual regression). Every Playwright screenshot test takes its
// screenshots through `expectScreenshot`, which waits for the page's fonts
// and records the Reference screenshot it reads, so the orphan check
// (`xtask screenshots orphans`) knows which references a run used. A
// reference compared without this helper is reported as an orphan.
import { appendFileSync, mkdirSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { expect, test, type Page } from "@playwright/test";

/** A Reference screenshot's name: a PNG file, without the platform. */
export type ScreenshotName = `${string}.png`;

/** Where each run records the references it read, one repository path a line. */
const READ_RECORDS = ["target", "visual", "read"] as const;

/** Compares the page with its Reference screenshot, pixel for pixel. */
export async function expectScreenshot(page: Page, name: ScreenshotName): Promise<void> {
  await page.evaluate(async () => {
    await document.fonts.ready;
  });
  recordReference(test.info().snapshotPath(name, { kind: "screenshot" }));
  await expect(page).toHaveScreenshot(name);
}

function recordReference(reference: string): void {
  const configFile = test.info().config.configFile;
  if (configFile === undefined) {
    throw new Error("the visual project runs from the repository's playwright.config.ts");
  }
  const root = dirname(configFile);
  const folder = join(root, ...READ_RECORDS);
  mkdirSync(folder, { recursive: true });
  const path = relative(root, reference).split(sep).join("/");
  appendFileSync(join(folder, `playwright-${process.pid}.txt`), `${path}\n`);
}
