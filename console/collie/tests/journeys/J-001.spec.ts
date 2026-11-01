// J-001, "Start the Daemon and open the local Console with no sign-in", on
// the `console` surface (ADRs 0024, 0031, 0063 and 0075): a person shortens
// the Daemon's idle time through `daemon.idle_timeout`, starts the Daemon,
// opens the local Console's address, signed in by the link `console`
// gives, then closes the Console, and the Daemon stops on its own once its
// idle time has passed with no Client connected.
import { readFile, writeFile } from "node:fs/promises";
import type { SettingsExplanation } from "@goodfetch/collie-client/generated";
import { journey } from "@goodfetch/test-support/journey";
import { expect } from "@goodfetch/test-support/playwright";

/** The idle time the person gives the Daemon, in milliseconds. */
const IDLE = 2000;

/** How long the Daemon may take to stop once its idle time began: that time, then its stop. */
const STOPS = 30_000;

/** The field `name` of `value`, if it is an object that has one. */
function field(value: unknown, name: string): unknown {
  return typeof value === "object" && value !== null
    ? Object.entries(value).find(([key]) => key === name)?.[1]
    : undefined;
}

journey("J-001", "console", async ({ step, check, cli, page }) => {
  /** What `daemon status --json` says of the test home's Daemon. */
  const status = async (): Promise<unknown> => {
    const output = await cli(["daemon", "status", "--json"]);
    expect(output.code, output.stderr).toBe(0);
    return JSON.parse(output.stdout);
  };

  await step(
    "Shorten the Daemon's idle time through its `daemon.idle_timeout` setting",
    async () => {
      const explained = await cli(["config", "explain", "daemon.idle_timeout", "--json"]);
      expect(explained.code, explained.stderr).toBe(0);
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `config explain --json` prints `goodfetch-protocol`'s type.
      const { files } = JSON.parse(explained.stdout) as SettingsExplanation;
      const user = files.find((file) => file.layer === "user_configuration");
      if (user === undefined) {
        throw new Error("config explain reads no user configuration");
      }
      const text = await readFile(user.path, "utf8");
      await writeFile(
        user.path,
        text.replace(
          /console\.address = "[^"]*"/,
          (line) => `${line}\ndaemon.idle_timeout = ${String(IDLE)}`,
        ),
      );
      const shortened = await cli(["config", "explain", "daemon.idle_timeout", "--json"]);
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- as above.
      const { settings } = JSON.parse(shortened.stdout) as SettingsExplanation;
      expect(settings.map((setting) => [setting.key, setting.value])).toEqual([
        ["daemon.idle_timeout", IDLE],
      ]);
    },
  );

  await step("Start the Daemon", async () => {
    // The variant's setup started a Daemon before the idle time was
    // shortened: the person starts it again, so it reads the new time.
    const stopped = await cli(["daemon", "stop"]);
    expect(stopped.code, stopped.stderr).toBe(0);
    const started = await cli(["daemon", "start"]);
    expect(started.code, started.stderr).toBe(0);
    expect(field(await status(), "pid")).not.toBeNull();
  });

  await step("Open the local Console's address", async () => {
    const opened = await cli(["console", "--json", "--no-browser"]);
    expect(opened.code, opened.stderr).toBe(0);
    const link = field(JSON.parse(opened.stdout), "url");
    if (typeof link !== "string") {
      throw new Error(`\`console --json\` gave no link: ${opened.stdout}`);
    }
    await page.goto(link);
    await page.waitForURL((url) => url.hash === "");
  });

  await check("The local Console opens with no sign-in", async () => {
    await expect(page.getByText("Connected to Collie", { exact: false })).toBeVisible();
    await expect(page.getByText("Not signed in", { exact: false })).toHaveCount(0);
  });

  await step("Close the Console and wait past the idle time", async () => {
    await page.close();
    // Each `daemon status` is a Client of its own: the checks come further
    // apart than the idle time, so the Daemon has its idle time between them.
    await expect
      .poll(async () => field(await status(), "pid"), { timeout: STOPS, intervals: [3 * IDLE] })
      .toBeNull();
  });

  await check(
    "The Daemon stops on its own once no Client has been connected for its idle time",
    async () => {
      expect(field(await status(), "pid")).toBeNull();
    },
  );
});
