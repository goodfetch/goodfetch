// A crawl run's Datasets on Foxhound's Daemon serving Foxhound's Console
// build (spec #593, stories 204, 205 and 233; ADRs 0031, 0071, 0094 and
// 0103), on Foxhound's executable built from the commit, with a Daemon
// and home of its own on port 0. The crawl is the Compatibility corpus's
// crawl head (ADR 0047), a crawl of the Proving ground's shop seeded as a
// run state copied from another machine and indexed with `runs reindex`:
// its Datasets list their schema, key and committed, dropped and rejected
// counts, and a Dataset's Records show in the grid. A Dataset of 10,000
// Records added to it shows in a grid that keeps only the rows on screen
// in the DOM, page after page read by key, to its last Record.
import type { DatabaseSync } from "node:sqlite";
import { expect, test } from "@goodfetch/test-support/playwright";
import { bodyRows, openDatasets } from "./seeded-crawl.ts";

/** How many Records the large Dataset holds. */
const ITEMS = 10_000;

/** Adds the Dataset `items`, declared with no schema, holding `count` Records. */
function withItems(count: number): (database: DatabaseSync) => void {
  return (database) => {
    database.exec("BEGIN");
    database.prepare("INSERT INTO datasets VALUES ('items', 2, NULL, NULL, '[]')").run();
    const insert = database.prepare(
      "INSERT INTO records VALUES ('items', ?, '1', 'https://shop.test/', '2026-01-05T09:00:00Z', ?, NULL, '{\"request_id\":\"1\"}')",
    );
    for (let number = 1; number <= count; number += 1) {
      insert.run(number, JSON.stringify({ n: number, name: `Item ${String(number)}` }));
    }
    database.exec("COMMIT");
  };
}

test.use({ product: "foxhound" });

test(
  "a crawl run's Datasets list their schema, key and committed, dropped and rejected counts, and a Dataset's Records show in the grid",
  {
    annotation: [
      { type: "covers", description: "story:12-204" },
      { type: "covers", description: "story:12-205" },
      { type: "covers", description: "screen:/runs/$run" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    await openDatasets(page, daemon, signIn);

    const pagesSchema =
      '{"properties":{"title":{"type":"string"},"url":{"type":"string"}},"required":["url"],"type":"object"}';
    await expect
      .poll(() => bodyRows(page, "Datasets"))
      .toEqual([
        ["pages", pagesSchema, "url", "4", "0", "1"],
        ["links", "Any JSON object", "None", "4", "0", "0"],
      ]);
    await expect
      .poll(() => bodyRows(page, "Records of pages"))
      .toEqual([1, 2, 3, 4].map((number) => [String(number), "Shop", "https://shop.test/"]));

    await page.getByRole("combobox", { name: "Sort by" }).selectOption("title");
    await page.getByRole("combobox", { name: "Order" }).selectOption("descending");
    await page.getByRole("checkbox", { name: "url" }).uncheck();
    await page.getByRole("button", { name: "Apply" }).click();
    await expect
      .poll(() => bodyRows(page, "Records of pages"))
      .toEqual([1, 2, 3, 4].map((number) => [String(number), "Shop"]));
  },
);

test(
  "a Records grid of 10,000 rows keeps only the rows on screen in the DOM, to the last Record",
  {
    annotation: [
      { type: "covers", description: "story:12-233" },
      { type: "covers", description: "story:12-205" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    await openDatasets(page, daemon, signIn, withItems(ITEMS));
    await page.getByRole("combobox", { name: "Records of the Dataset" }).selectOption("items");
    const grid = page.getByRole("table", { name: "Records of items" });
    const rendered = grid.locator("tbody tr:not([aria-hidden])");
    await expect(rendered.first()).toBeVisible();

    const read = page.locator("output").filter({ hasText: /Records? read/ });
    const counts: number[] = [];
    const pagesRead: string[] = [];
    /** How far the grid has read: its count, and whether more are to read. */
    const progress = async (): Promise<string> =>
      `${(await read.textContent()) ?? ""} ${(await grid.getAttribute("aria-rowcount")) ?? ""}`;
    // Each scroll to the page's end brings the last rows read into view, and
    // the grid reads the next page, one after the other: the last, past a
    // full page, holds none and ends the Dataset.
    while ((await grid.getAttribute("aria-rowcount")) === "-1") {
      const before = await progress();
      pagesRead.push((await read.textContent()) ?? "");
      counts.push(await rendered.count());
      await page.evaluate(() => window.scrollTo({ top: document.documentElement.scrollHeight }));
      await expect.poll(progress).not.toBe(before);
    }
    await page.evaluate(() => window.scrollTo({ top: document.documentElement.scrollHeight }));
    await expect(grid).toHaveAttribute("aria-rowcount", String(ITEMS + 1));
    await expect(read).toHaveText("10,000 Records read");
    await expect(grid.getByRole("rowheader", { name: String(ITEMS), exact: true })).toBeVisible();
    counts.push(await rendered.count());

    expect(pagesRead).toEqual(
      Array.from(
        { length: ITEMS / 500 },
        (_, index) => `${String((index + 1) * 500)} Records read`,
      ).map((text) => text.replace(/\B(?=(\d{3})+(?!\d))/g, ",")),
    );
    expect(Math.max(...counts)).toBeLessThan(100);
  },
);
