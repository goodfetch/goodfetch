// A run's Run report served by Collie's executable (spec #592; story 15;
// ADRs 0028, 0041 and 0102), each test with its own Daemon and home on port
// 0: the page of a run stopped by a Limit names that Limit as its stop
// reason, the Limits that apply to it and what is left of them, and the
// page of a run whose settings weakened a safeguard names the setting, its
// value and the layer that set it.
//
// The served Daemon has no browser, and a Collie run charges Usage only
// through one, so no run it starts can reach a Limit. Both runs are the
// Compatibility corpus's run-state head of a stopped run (ADR 0047), which
// ran under `tls.accept_invalid_certs` set in its settings, written into
// the per-major data folder's run folders, the Limit stop given to one of
// them, and indexed with `runs reindex`, as a run state copied from another
// machine is: the Daemon reads and answers their reports as it would its
// own.
import { mkdir, readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { expect, test, type ServedDaemon } from "@goodfetch/test-support/playwright";
import type { Page } from "@playwright/test";

/** The corpus's run-state head of a run a Client aborted. */
const ABORTED = new URL("../../../../compat/run-state/head/aborted.sql", import.meta.url);

/** The stop of a run whose install Limit on requests had nothing left. */
const LIMIT_STOP = {
  kind: "limit",
  scope: { kind: "install", id: "" },
  usage_kind: "requests",
  period: { every: "month", time_zone: "UTC" },
  resets_at: "2026-02-01T00:00:00Z",
};

/** Writes the corpus's stopped run as `runId` under `folder`, stopped by `stop` when given. */
async function seeded(folder: string, runId: string, stop?: object): Promise<void> {
  const runFolder = join(folder, runId);
  await mkdir(runFolder, { recursive: true });
  const database = new DatabaseSync(join(runFolder, "run.sqlite"));
  try {
    database.exec(await readFile(ABORTED, "utf8"));
    const row = database.prepare("SELECT report FROM run").get();
    const report: Record<string, unknown> = JSON.parse(String(row?.["report"]));
    report["run_id"] = runId;
    if (stop !== undefined) {
      report["stop_reason"] = stop;
      report["completeness_reasons"] = [
        { code: "stopped", message: "It stopped early: a Limit on install requests was reached." },
      ];
    }
    database
      .prepare("UPDATE run SET run_id = ?, stop_reason = ?, report = ?")
      .run(runId, JSON.stringify(report["stop_reason"]), JSON.stringify(report));
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

/** Runs the executable's `args` in the test's home, failing the test when it fails. */
async function run(daemon: ServedDaemon, args: readonly string[]): Promise<void> {
  const output = await daemon.run(args);
  expect(output.code, output.stderr).toBe(0);
}

/** The report's section headed `name`. */
function section(page: Page, name: string) {
  return page.getByRole("region", { name, exact: true });
}

test(
  "the reports of a run stopped by a Limit and of one that weakened a safeguard name the Limit and the setting",
  {
    annotation: [
      { type: "covers", description: "story:12-15" },
      { type: "covers", description: "screen:/runs/$run" },
      { type: "covers", description: "operation:run.report" },
      { type: "covers", description: "command:runs.reindex" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    const runs = await runsFolder(daemon);
    await seeded(runs, "limited-run", LIMIT_STOP);
    await seeded(runs, "weakened-run");
    await run(daemon, ["runs", "reindex"]);
    await signIn.withLink(page);

    await page.goto(new URL("/runs/limited-run", daemon.url).href);

    const ending = section(page, "How it ended");
    await expect(ending).toContainText(
      "Stop reasonA Limit on install capping requests was reached.",
    );
    await expect(
      section(page, "Usage against its Limits").getByRole("table", {
        name: "Limits that apply to this run",
      }),
    ).toContainText("requests");
    await expect(section(page, "Model allowance")).toContainText(
      "No Model allowance is set for this run.",
    );

    await page.goto(new URL("/runs/weakened-run", daemon.url).href);

    await expect(page.getByRole("list", { name: "Settings that weakened a safeguard" })).toHaveText(
      'tls.accept_invalid_certs = {"intranet.test":null}, set by Run, from the run\'s settings',
    );
    await expect(section(page, "How it ended")).toContainText("Aborted by a Client.");
  },
);
