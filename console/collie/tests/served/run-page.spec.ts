// A run's page served by Collie's executable (spec #592; ADRs 0029, 0102
// and 0122), each test with its own Daemon and home on a named port: the
// `consoleUrl` of a Run report opens it, an unknown Run ID says the Run
// index does not list it, a live run's page writes a Checkpoint and stops
// the run, then copies the command that resumes it, a run whose Daemon was
// killed shows no writer and its resume command, and, on a size-capped
// filesystem, the page shows the worst storage level with its folder and
// free space as the level changes.
import { appendFile, readdir } from "node:fs/promises";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, test, type ServedDaemon } from "@goodfetch/test-support/playwright";

// The test of a killed Daemon starts it again under its page's address.
test.use({ namedConsolePort: true });

/** Runs the executable's `args` in the test's home, answering what it printed; a failure fails the test unless its exit code is `allowed`. */
async function run(daemon: ServedDaemon, args: readonly string[], allowed = 0): Promise<string> {
  const output = await daemon.run(args);
  expect([0, allowed], output.stderr).toContain(output.code);
  return output.stdout;
}

/** The field `name` of the JSON object `text` holds. */
function fieldOf(text: string, name: string): unknown {
  const value: unknown = JSON.parse(text);
  return typeof value === "object" && value !== null && name in value
    ? Object.entries(value).find(([key]) => key === name)?.[1]
    : undefined;
}

/** Each term the page shows, with its value's text. */
async function facts(page: Page): Promise<Record<string, string>> {
  const terms = await page.getByRole("term").allTextContents();
  const values = await page.getByRole("definition").allTextContents();
  return Object.fromEntries(terms.map((term, index) => [term, values[index] ?? ""]));
}

test(
  "the consoleUrl of a Run report opens its run's page, with its three axes as separate values",
  {
    annotation: [
      { type: "covers", description: "story:12-16" },
      { type: "covers", description: "story:12-11" },
      { type: "covers", description: "screen:/runs/$run" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    await run(daemon, ["run", "start", "--run-id", "done-run"]);
    const report = await run(daemon, ["run", "finish", "done-run", "--json"]);
    const consoleUrl = fieldOf(report, "console_url");
    expect(consoleUrl).toBe(new URL("/runs/done-run", daemon.url).href);
    await signIn.withLink(page);

    await page.goto(String(consoleUrl));

    await expect(page.getByRole("heading", { level: 1 })).toHaveText("Run done-run");
    await expect
      .poll(() => facts(page))
      .toMatchObject({ Lifecycle: "finished", Verdict: "verified", Completeness: "complete" });
    await expect(page).toHaveTitle("Run · Goodfetch Collie");
  },
);

test(
  "the page of a Run ID the Run index does not list says so",
  {
    annotation: [
      { type: "covers", description: "story:12-16" },
      { type: "covers", description: "screen:/runs/$run" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    await signIn.withLink(page);

    await page.goto(new URL("/runs/no-such-run", daemon.url).href);

    await expect(page.getByRole("alert")).toHaveText(
      "No run has the Run ID no-such-run: the Run index does not list it.",
    );
  },
);

test(
  "a live run's page writes a Checkpoint and stops it, then copies the command that resumes it",
  {
    annotation: [
      { type: "covers", description: "story:12-20" },
      { type: "covers", description: "story:12-19" },
      { type: "covers", description: "story:12-14" },
      { type: "covers", description: "screen:/runs/$run" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    await run(daemon, ["run", "start", "--run-id", "live-run"]);
    await signIn.withLink(page);
    await page.goto(new URL("/runs/live-run", daemon.url).href);
    await expect
      .poll(() => facts(page))
      .toMatchObject({ Writer: "live: an engine works this run", "Last Checkpoint": "none yet" });

    await page.getByRole("button", { name: "Write a Checkpoint" }).click();

    await expect(page.getByRole("status").filter({ hasText: "Checkpoint written" })).toBeVisible();
    await expect.poll(async () => (await facts(page))["Last Checkpoint"]).not.toBe("none yet");

    await page.getByRole("button", { name: "Stop" }).click();

    await expect
      .poll(() => facts(page))
      .toMatchObject({ Lifecycle: "stopped", Completeness: "incomplete" });
    await expect(page.getByRole("button", { name: "Stop" })).toHaveCount(0);
    const command = "goodfetch-collie run resume live-run";
    await expect(page.getByText(command, { exact: true })).toBeVisible();
    await page.context().grantPermissions(["clipboard-read", "clipboard-write"], {
      origin: new URL(daemon.url).origin,
    });

    await page.getByRole("button", { name: "Copy resume command" }).click();

    await expect(page.getByRole("status").filter({ hasText: "resume command" })).toHaveText(
      "Copied the resume command.",
    );
    expect(await page.evaluate(async () => navigator.clipboard.readText())).toBe(command);
  },
);

/** Kills the test's Daemon, as a crash does, then starts it again on its Console's named port. */
async function crashedAndRestarted(daemon: ServedDaemon): Promise<void> {
  const pid = Number(fieldOf(await run(daemon, ["daemon", "status", "--json"]), "pid"));
  expect(pid).toBeGreaterThan(0);
  process.kill(pid, "SIGKILL");
  // Its process may stay a zombie until its parent reaps it; it answers
  // no Client once it died.
  await expect
    .poll(async () => fieldOf(await run(daemon, ["daemon", "status", "--json"]), "pid"))
    .toBeNull();
  await run(daemon, ["daemon", "start"]);
}

test(
  "an interrupted run's page shows no writer, offers no stop and shows the command that resumes it",
  {
    annotation: [
      { type: "covers", description: "story:12-20" },
      { type: "covers", description: "story:12-19" },
      { type: "covers", description: "screen:/runs/$run" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    await run(daemon, ["run", "start", "--run-id", "crashed-run"]);
    await crashedAndRestarted(daemon);
    await signIn.withLink(page);

    await page.goto(new URL("/runs/crashed-run", daemon.url).href);

    await expect
      .poll(() => facts(page))
      .toMatchObject({ Lifecycle: "running", Writer: "none: no engine works this run" });
    await expect(page.getByText("it was interrupted", { exact: false })).toBeVisible();
    await expect(
      page.getByText("goodfetch-collie run resume crashed-run", { exact: true }),
    ).toBeVisible();
    await expect(page.getByRole("button", { name: "Stop" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Copy resume command" })).toBeVisible();
  },
);

const MIB = 1 << 20;

/** The size-capped filesystem's size, and the two levels against it. */
const SIZE = 64 * MIB;
const LEVELS = `storage.low_free = ${String(40 * MIB)}\nstorage.critical_free = ${String(20 * MIB)}\n`;

/** Free space between the two levels. */
const LOW_ROOM = 30 * MIB;

/** How long a level change may take to reach the page: a sample a second. */
const SAMPLED = 30_000;

test.describe("on a size-capped filesystem", () => {
  test.skip(
    process.platform !== "linux",
    "a size-capped filesystem is a tmpfs on Linux; macOS's and Windows's small disk image is not made yet",
  );
  test.use({ sizeCapped: SIZE });

  test(
    "a run's page shows the worst storage level, with that filesystem's folder and free space, as it changes",
    {
      annotation: [
        { type: "covers", description: "story:12-19" },
        { type: "covers", description: "screen:/runs/$run" },
      ],
    },
    async ({ page, daemon, signIn }) => {
      const capped = daemon.sizeCapped;
      expect(capped).toBeDefined();
      if (capped === undefined) {
        return;
      }
      const home = capped.outside(daemon.home);
      for (const entry of await readdir(home, { withFileTypes: true })) {
        if (
          entry.isDirectory() &&
          (await readdir(join(home, entry.name))).includes("configuration.toml")
        ) {
          await appendFile(join(home, entry.name, "configuration.toml"), LEVELS);
        }
      }
      await run(daemon, ["run", "start", "--run-id", "filled"]);
      await signIn.withLink(page);
      await page.goto(new URL("/runs/filled", daemon.url).href);
      await expect.poll(async () => (await facts(page))["Storage"]).toBe("normal");

      await capped.fill(daemon.home, LOW_ROOM);

      const storage = page.getByRole("definition").filter({ hasText: "the filesystem of" });
      await expect(storage).toContainText("low", { timeout: SAMPLED });
      await expect(storage).toContainText(daemon.home);
      const free = Number(await storage.locator("data").getAttribute("value"));
      expect(free).toBeGreaterThan(0);
      expect(free).toBeLessThan(40 * MIB);

      await capped.free();

      await expect(storage).toContainText(/^normal/, { timeout: SAMPLED });
      await expect(storage).toContainText(daemon.home);
    },
  );
});
