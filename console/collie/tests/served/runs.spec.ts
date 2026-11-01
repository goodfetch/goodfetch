// The Runs screen served by Collie's executable (spec #592; ADRs 0031, 0036
// and 0117), each test with its own Daemon and home on port 0. Runs are
// seeded in every state a Client can leave them in: live, finished and
// stopped through the CLI, missing by removing a run's folder and
// reindexing, and erased by `run.discard` over the Daemon's socket, as an
// SDK calls it. The filter is opened from the address and survives a
// reload; a filter set on the screen is written to the address; and a live
// run's row follows its status line and its end without a reload.
import { execFile } from "node:child_process";
import { readdir, rm, stat } from "node:fs/promises";
import { createConnection } from "node:net";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { promisify } from "node:util";
import type { Page } from "@playwright/test";
import { expect, test, type ServedDaemon } from "@goodfetch/test-support/playwright";

const execute = promisify(execFile);

/** Runs the executable of the test's Daemon with `args` in its home, answering what it prints. */
async function command(daemon: ServedDaemon, args: readonly string[]): Promise<string> {
  const { stdout } = await execute(daemon.executable, [...args], { env: { ...daemon.env } });
  return stdout;
}

/** Calls `method` with `params` on the Daemon over its socket, as an SDK does, after `hello`. */
async function callDaemon(daemon: ServedDaemon, method: string, params: unknown): Promise<unknown> {
  const info: unknown = JSON.parse(await command(daemon, ["engine", "info", "--json"]));
  const version =
    typeof info === "object" && info !== null && "product_version" in info
      ? String(info.product_version)
      : "";
  const socket = createConnection(daemon.socket);
  const lines = createInterface({ input: socket })[Symbol.asyncIterator]();
  const send = (id: number, name: string, values: unknown): void => {
    socket.write(`${JSON.stringify({ jsonrpc: "2.0", id, method: name, params: values })}\n`);
  };
  const answer = async (id: number): Promise<unknown> => {
    for (;;) {
      const line = await lines.next();
      if (line.done === true) {
        throw new Error(`the Daemon closed before answering ${method}`);
      }
      const message: unknown = JSON.parse(line.value);
      if (typeof message === "object" && message !== null && "id" in message && message.id === id) {
        if ("error" in message) {
          throw new Error(`${method} failed: ${JSON.stringify(message.error)}`);
        }
        return "result" in message ? message.result : undefined;
      }
    }
  };
  try {
    send(0, "hello", {
      protocol_version: "0.1",
      surface: "sdk_typescript",
      token: daemon.token,
      needs: "collie",
      product_version: version,
    });
    await answer(0);
    send(1, method, params);
    return await answer(1);
  } finally {
    socket.end();
  }
}

/** The folder of the run `runId` in the test's home. */
async function runFolder(daemon: ServedDaemon, runId: string): Promise<string> {
  for (const major of await readdir(daemon.home)) {
    const folder = join(daemon.home, major, "runs", runId);
    if (
      await stat(folder).then(
        () => true,
        () => false,
      )
    ) {
      return folder;
    }
  }
  throw new Error(`no folder of the run ${runId} in ${daemon.home}`);
}

/** Seeds a run in each state: live, finished, stopped, missing and erased, in that order of start. */
async function seed(daemon: ServedDaemon): Promise<void> {
  await command(daemon, ["run", "start", "--run-id", "live-run", "--tags", "team:shop"]);
  await command(daemon, ["run", "start", "--run-id", "finished-run"]);
  await command(daemon, ["run", "finish", "finished-run"]);
  await command(daemon, ["run", "start", "--run-id", "stopped-run"]);
  // A command answering a stopped Run's report exits 20 (`run_stopped`).
  await command(daemon, ["run", "abort", "stopped-run"]).catch((error: unknown) => {
    if (typeof error !== "object" || error === null || !("code" in error) || error.code !== 20) {
      throw error;
    }
    return "";
  });
  await command(daemon, ["run", "start", "--run-id", "missing-run"]);
  await command(daemon, ["run", "finish", "missing-run"]);
  await rm(await runFolder(daemon, "missing-run"), { recursive: true });
  await command(daemon, ["runs", "reindex"]);
  await command(daemon, ["run", "start", "--run-id", "erased-run"]);
  await command(daemon, ["run", "finish", "erased-run"]);
  await callDaemon(daemon, "run.discard", { run_id: "erased-run" });
}

/** The Run IDs the table lists, in its order. */
async function listedIds(page: Page): Promise<string[]> {
  return page.getByRole("rowheader").locator("code").allTextContents();
}

/** The text of the row of `runId`. */
function rowOf(page: Page, runId: string) {
  return page.getByRole("row").filter({ has: page.locator("code", { hasText: runId }) });
}

test(
  "the filter opened from the address survives a reload, and missing and erased runs are listed as such",
  {
    annotation: [
      { type: "covers", description: "story:12-10" },
      { type: "covers", description: "story:12-12" },
      { type: "covers", description: "story:12-13" },
      { type: "covers", description: "screen:/runs" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    await seed(daemon);
    await signIn.withLink(page);

    await page.goto(
      new URL("/runs?lifecycle=finished&sort=run_id&order=ascending", daemon.url).href,
    );
    await expect.poll(() => listedIds(page)).toEqual(["erased-run", "finished-run", "missing-run"]);
    await expect(rowOf(page, "missing-run")).toContainText("Run file missing");
    await expect(rowOf(page, "erased-run")).toContainText("Erased");
    await expect(page.getByRole("combobox", { name: "Lifecycle" })).toHaveValue("finished");

    await page.reload();

    await expect.poll(() => listedIds(page)).toEqual(["erased-run", "finished-run", "missing-run"]);
    await expect(page.getByRole("combobox", { name: "Sort by" })).toHaveValue("run_id");

    await page.goto(new URL("/runs?sort=run_id&order=ascending", daemon.url).href);
    await expect
      .poll(() => listedIds(page))
      .toEqual(["erased-run", "finished-run", "live-run", "missing-run", "stopped-run"]);
    await expect(rowOf(page, "stopped-run")).toContainText("stopped");
    await expect(rowOf(page, "live-run")).toContainText("running");
    await expect(rowOf(page, "live-run")).toContainText("team:shop");
  },
);

test(
  "a filter set on the screen goes into the address, and a live run's row follows its status line and its end",
  {
    annotation: [
      { type: "covers", description: "story:12-12" },
      { type: "covers", description: "story:12-14" },
      { type: "covers", description: "screen:/runs" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    await command(daemon, ["run", "start", "--run-id", "live-run"]);
    await command(daemon, ["run", "start", "--run-id", "other-run"]);
    await signIn.withLink(page);
    await page.goto(new URL("/runs?sort=run_id&order=ascending", daemon.url).href);
    await expect.poll(() => listedIds(page)).toEqual(["live-run", "other-run"]);

    await page.getByRole("searchbox", { name: "Search" }).fill("live");
    await page.getByRole("button", { name: "Apply" }).click();

    await expect.poll(() => listedIds(page)).toEqual(["live-run"]);
    expect(new URL(page.url()).search).toBe("?text=live&sort=run_id&order=ascending");

    await callDaemon(daemon, "run.set_progress_message", {
      run_id: "live-run",
      message: "Read 3 of 3 pages",
    });
    await expect(rowOf(page, "live-run")).toContainText("Read 3 of 3 pages");
    await command(daemon, ["run", "finish", "live-run"]);
    await expect(rowOf(page, "live-run")).toContainText("finished");
    await expect(rowOf(page, "live-run")).toContainText("complete");
  },
);
