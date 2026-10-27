// A crawl run for Foxhound's served tests: the Compatibility corpus's crawl
// head (ADR 0047), a crawl of the Proving ground's shop, seeded as a run
// state copied from another machine and indexed with `runs reindex`, so its
// run page shows its Datasets.
import { mkdir, readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { expect, type ServedDaemon } from "@goodfetch/test-support/playwright";
import type { Page } from "@playwright/test";

/** The corpus's run-state heads. */
const CORPUS = new URL("../../../../compat/run-state/head/", import.meta.url);

/** The Run ID the tests give the corpus's crawl. */
export const CRAWL_RUN = "copied-crawl";

/** Writes the corpus's crawl head as `runId` under `folder`, ended, with `extra` run against its run file. */
async function seeded(
  folder: string,
  runId: string,
  extra: (database: DatabaseSync) => void = () => {},
): Promise<void> {
  const runFolder = join(folder, runId);
  await mkdir(runFolder, { recursive: true });
  const database = new DatabaseSync(join(runFolder, "run.sqlite"));
  try {
    database.exec(await readFile(new URL("crawl.sql", CORPUS), "utf8"));
    const row = database.prepare("SELECT report FROM run").get();
    const kept = row?.["report"];
    let report: string | null = null;
    if (typeof kept === "string") {
      const parsed: Record<string, unknown> = JSON.parse(kept);
      parsed["run_id"] = runId;
      report = JSON.stringify(parsed);
    }
    database.prepare("UPDATE run SET run_id = ?, report = ?").run(runId, report);
    extra(database);
  } finally {
    database.close();
  }
}

/** The run folders of the per-major data folder, the one that holds the Daemon's socket. */
async function runsFolder(daemon: ServedDaemon): Promise<string> {
  for (const entry of await readdir(daemon.home, { withFileTypes: true })) {
    const folder = join(daemon.home, entry.name);
    if (entry.isDirectory() && (await readdir(folder)).includes("daemon.sock")) {
      return join(folder, "runs");
    }
  }
  throw new Error("the test's home holds no per-major folder with the Daemon's socket");
}

/** Seeds the crawl as `CRAWL_RUN`, with `extra` run against its run file, indexes it, signs in and opens its Datasets tab. */
export async function openDatasets(
  page: Page,
  daemon: ServedDaemon,
  signIn: { withLink: (page: Page) => Promise<void> },
  extra?: (database: DatabaseSync) => void,
): Promise<void> {
  await seeded(await runsFolder(daemon), CRAWL_RUN, extra);
  const reindexed = await daemon.run(["runs", "reindex"]);
  expect(reindexed.code, reindexed.stderr).toBe(0);
  await signIn.withLink(page);
  await page.goto(new URL(`/runs/${CRAWL_RUN}`, daemon.url).href);
  await page.getByRole("tab", { name: "Datasets", exact: true }).click();
}

/** The text of each cell of `table`'s body rows that hold a Record. */
export async function bodyRows(page: Page, name: string): Promise<string[][]> {
  return page
    .getByRole("table", { name })
    .locator("tbody tr:not([aria-hidden])")
    .evaluateAll((rows) =>
      rows.map((row) => [...row.querySelectorAll("th, td")].map((cell) => cell.textContent)),
    );
}
