// A run that needs Foxhound, on Collie's Daemon (spec #591, stories 120 and
// 121; ADR 0098), served by Collie's executable built from the commit, with
// a Daemon and home of its own on port 0. No Foxhound executable is needed:
// the run is the Compatibility corpus's run-state head of a crawl (ADR
// 0047), written into the per-major data folder's run folders and indexed
// with `runs reindex`, as a run state copied from another machine is. The
// Runs screen lists it with the Product it needs; its page keeps the views
// every run has, and each Foxhound view shows the `not_in_product` Collie's
// engine answers, whose fix names Foxhound.
import { mkdir, readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { expect, test, type ServedDaemon } from "@goodfetch/test-support/playwright";

/** The corpus's run-state head of a finished crawl. */
const CRAWL = new URL("../../../../compat/run-state/head/crawl.sql", import.meta.url);

/** The Run ID the test gives the corpus's crawl. */
const RUN_ID = "copied-crawl";

/** Writes the corpus's crawl as `runId` under `folder`. */
async function seeded(folder: string, runId: string): Promise<void> {
  const runFolder = join(folder, runId);
  await mkdir(runFolder, { recursive: true });
  const database = new DatabaseSync(join(runFolder, "run.sqlite"));
  try {
    database.exec(await readFile(CRAWL, "utf8"));
    const row = database.prepare("SELECT report FROM run").get();
    const report: Record<string, unknown> = JSON.parse(String(row?.["report"]));
    report["run_id"] = runId;
    database.prepare("UPDATE run SET run_id = ?, report = ?").run(runId, JSON.stringify(report));
  } finally {
    database.close();
  }
}

/**
 * The run folders of the per-major data folder, the one that holds the
 * Daemon's runtime file: its socket is elsewhere when its path there would
 * be too long for the OS.
 */
async function runsFolder(daemon: ServedDaemon): Promise<string> {
  for (const entry of await readdir(daemon.home, { withFileTypes: true })) {
    const folder = join(daemon.home, entry.name);
    if (entry.isDirectory() && (await readdir(folder)).includes("daemon.json")) {
      return join(folder, "runs");
    }
  }
  throw new Error("the test's home holds no per-major folder with the Daemon's runtime file");
}

test(
  "on Collie's Daemon, a run that needs Foxhound keeps its common views and shows not_in_product in each Foxhound tab",
  {
    annotation: [
      { type: "covers", description: "story:12-120" },
      { type: "covers", description: "story:12-121" },
      { type: "covers", description: "screen:/runs" },
      { type: "covers", description: "screen:/runs/$run" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    await seeded(await runsFolder(daemon), RUN_ID);
    const reindexed = await daemon.run(["runs", "reindex"]);
    expect(reindexed.code, reindexed.stderr).toBe(0);
    await signIn.withLink(page);

    await page.goto(new URL("/runs", daemon.url).href);
    const row = page.getByRole("row").filter({ hasText: RUN_ID });
    await expect(row.getByRole("cell", { name: "Foxhound", exact: true })).toBeVisible();

    await row.getByRole("link", { name: RUN_ID }).click();

    const tabs = page.getByRole("tablist", { name: "Run views" }).getByRole("tab");
    await expect(tabs).toHaveText([
      "Run",
      "Records",
      "Outcomes",
      "Failed requests",
      "Seeds",
      "Rejected records",
    ]);
    await expect(page.getByRole("region", { name: "How it stands" })).toContainText("finished");
    await expect(page.getByRole("term").filter({ hasText: "Product it needs" })).toBeVisible();

    for (const name of ["Records", "Outcomes", "Failed requests", "Seeds", "Rejected records"]) {
      await page.getByRole("tab", { name, exact: true }).click();
      const panel = page.getByRole("tabpanel", { name, exact: true });
      await expect(panel).toContainText("This engine's Product does not carry what was asked.");
      await expect(panel).toContainText("Run it with Foxhound");
      await expect(panel).toContainText("not_in_product");
    }
  },
);
