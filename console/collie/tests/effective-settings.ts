// Engine & settings' effective settings as the served Console's Playwright
// tests and Journeys reach them: a WAI-ARIA grid named by its caption, one
// row per setting with the key as its `rowheader` and the value, the layer
// that set it, the layer that locked it and its notes as `gridcell`s.
import type { Locator, Page } from "@playwright/test";

/** The effective settings grid of Engine & settings. */
export function effectiveSettings(page: Page): Locator {
  return page.getByRole("grid", { name: "Effective settings" });
}

/** The effective settings' row of the setting `key`. */
export function settingRow(page: Page, key: string): Locator {
  return effectiveSettings(page)
    .getByRole("row")
    .filter({ has: page.getByRole("rowheader", { name: key, exact: true }) });
}
