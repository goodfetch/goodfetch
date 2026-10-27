// Foxhound's Daemon serving Foxhound's Console build (spec #593, stories
// 201 and 203; ADRs 0094 and 0098), on Foxhound's executable built from the
// commit, with a Daemon and home of its own on port 0. The runs are the
// Compatibility corpus's run-state heads (ADR 0047) of a Collie run and of
// a crawl, each left stopped by a filled filesystem, written into the
// per-major data folder's run folders and indexed with `runs reindex`, as
// run states copied from another machine are. One Runs screen lists both
// Products' runs, one "Needs you" lists the storage stop of each, the
// Collie run's page shows only the views every run has, and the crawl's
// shows Foxhound's views beside them, each answered by Foxhound's engine,
// its SQL tab included.
import { mkdir, readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { expect, test, type ServedDaemon } from "@goodfetch/test-support/playwright";

/** The corpus's run-state heads. */
const CORPUS = new URL("../../../../compat/run-state/head/", import.meta.url);

/** The Run ID the test gives the corpus's Collie run, and its crawl. */
const COLLIE_RUN = "copied-collie-run";
const CRAWL_RUN = "copied-crawl";

/** Writes the corpus's run `head` as `runId` under `folder`, stopped by a filled filesystem. */
async function seeded(folder: string, head: string, runId: string): Promise<void> {
  const runFolder = join(folder, runId);
  await mkdir(runFolder, { recursive: true });
  const database = new DatabaseSync(join(runFolder, "run.sqlite"));
  try {
    database.exec(await readFile(new URL(`${head}.sql`, CORPUS), "utf8"));
    // The Run index takes a run's stop from its Run report, which says it too.
    const stop = { kind: "resources", resource: "storage", folder };
    const row = database.prepare("SELECT report FROM run").get();
    const kept = row?.["report"];
    let report: string | null = null;
    if (typeof kept === "string") {
      const parsed: Record<string, unknown> = JSON.parse(kept);
      parsed["run_id"] = runId;
      parsed["lifecycle"] = "stopped";
      parsed["stop_reason"] = stop;
      parsed["verdict"] = "verified";
      parsed["completeness"] = "incomplete";
      report = JSON.stringify(parsed);
    }
    database
      .prepare(
        "UPDATE run SET run_id = ?, lifecycle = 'stopped', stop_reason = ?, verdict = 'verified', completeness = 'incomplete', report = ?",
      )
      .run(runId, JSON.stringify(stop), report);
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

/** How long a run index change may take to reach "Needs you": its sources read again at each storage sample, a second apart, under load later. */
const SAMPLED = 30_000;

test.use({ product: "foxhound" });

test(
  "Foxhound's Daemon serves Foxhound's build: both Products' runs in one Runs screen and one Needs you, and Foxhound's views for the run that needs Foxhound only",
  {
    annotation: [
      { type: "covers", description: "story:12-201" },
      { type: "covers", description: "story:12-203" },
      { type: "covers", description: "story:12-210" },
      { type: "covers", description: "screen:/runs" },
      { type: "covers", description: "screen:/runs/$run" },
      { type: "covers", description: "screen:/needs-you" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    const runs = await runsFolder(daemon);
    await seeded(runs, "paused", COLLIE_RUN);
    await seeded(runs, "crawl", CRAWL_RUN);
    const reindexed = await daemon.run(["runs", "reindex"]);
    expect(reindexed.code, reindexed.stderr).toBe(0);
    await signIn.withLink(page);

    await expect(page).toHaveTitle("Goodfetch Foxhound");
    await expect(page.getByRole("main").getByRole("heading", { level: 1 })).toHaveText(
      "Goodfetch Foxhound",
    );

    await page
      .getByRole("navigation", { name: "Console" })
      .getByRole("link", { name: "Runs" })
      .click();
    const collieRow = page.getByRole("row").filter({ hasText: COLLIE_RUN });
    const crawlRow = page.getByRole("row").filter({ hasText: CRAWL_RUN });
    await expect(collieRow.getByRole("cell", { name: "Collie", exact: true })).toBeVisible();
    await expect(crawlRow.getByRole("cell", { name: "Foxhound", exact: true })).toBeVisible();

    await page
      .getByRole("navigation", { name: "Console" })
      .getByRole("link", { name: /Needs you/ })
      .click();
    const items = page.getByRole("list", { name: "Needs you" }).getByRole("listitem");
    await expect(items.filter({ hasText: COLLIE_RUN })).toHaveCount(1, { timeout: SAMPLED });
    await expect(items.filter({ hasText: CRAWL_RUN })).toHaveCount(1, { timeout: SAMPLED });

    await page.goto(new URL(`/runs/${COLLIE_RUN}`, daemon.url).href);
    await expect(page.getByRole("region", { name: "How it stands" })).toContainText("stopped");
    await expect(page.getByRole("tablist")).toHaveCount(0);

    await page.goto(new URL(`/runs/${CRAWL_RUN}`, daemon.url).href);
    const tabs = page.getByRole("tablist", { name: "Run views" }).getByRole("tab");
    await expect(tabs).toHaveText([
      "Run",
      "Progress",
      "Seeds",
      "Outcomes",
      "Failed requests",
      "Datasets",
      "SQL",
    ]);
    await expect(page.getByRole("region", { name: "How it stands" })).toContainText("stopped");
    for (const name of ["Seeds", "Outcomes", "Failed requests"]) {
      await page.getByRole("tab", { name, exact: true }).click();
      const panel = page.getByRole("tabpanel", { name, exact: true });
      await expect(panel.getByRole("table")).toBeVisible();
      await expect(panel).not.toContainText("not_in_product");
    }
    await page.getByRole("tab", { name: "Seeds", exact: true }).click();
    await expect(page.getByRole("tabpanel", { name: "Seeds" })).toContainText("https://shop.test/");

    // The SQL tab runs its statement through Foxhound's engine (story 210),
    // which counts the Records the crawl's run file holds.
    const crawlState = new DatabaseSync(join(runs, CRAWL_RUN, "run.sqlite"), { readOnly: true });
    const committed = crawlState.prepare("SELECT count(*) AS n FROM records").get()?.["n"];
    crawlState.close();
    await page.getByRole("tab", { name: "SQL", exact: true }).click();
    const sql = page.getByRole("tabpanel", { name: "SQL" });
    await sql
      .getByLabel("Statement")
      .fill(`SELECT count(*) AS committed FROM records('${CRAWL_RUN}')`);
    await sql.getByRole("button", { name: "Run the statement" }).click();
    await expect(sql.getByRole("columnheader")).toHaveText(["committed BIGINT"]);
    await expect(sql.getByRole("cell")).toHaveText([String(committed)]);
  },
);
