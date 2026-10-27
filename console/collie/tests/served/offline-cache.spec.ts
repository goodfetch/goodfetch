// The offline cache (spec #592's `offline`; ADR 0031), on Collie's
// executable built from the commit, under its production headers, each test
// with a Daemon and home of its own on port 0. A page keeps the run
// summaries it reads in IndexedDB, encrypted under a key only its sign-in's
// cookie unwraps, and goes on showing them, moving between its screens,
// after the Daemon stops and the browser goes offline; Sign out wipes the
// cache; once the sign-in is revoked, the Daemon unwraps its key no more
// and a page signed out wipes what is left; `console.offline_cache` set to
// `false` writes nothing; and Secret canaries planted in a run's data never
// reach the cache, neither in its records nor once decrypted.
import { readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { searchCanaries, type Canary, type SecretKind } from "@goodfetch/test-support/canaries";
import { expect, test, type ServedDaemon } from "@goodfetch/test-support/playwright";
import { callDaemon, command } from "@goodfetch/test-support/daemon-calls";
import {
  decryptedOfflineCache as decrypted,
  revokeOwnSignIn,
  storedOfflineCache as stored,
} from "@goodfetch/test-support/offline-cache";

/** The banner of a page with no Connection. */
const BANNER =
  "No connection to the engine: this page is read-only until the Console reconnects, which it does by itself.";

/** Starts the run `runId` and finishes it. */
async function finishedRun(daemon: ServedDaemon, runId: string): Promise<void> {
  await command(daemon, ["run", "start", "--run-id", runId]);
  await command(daemon, ["run", "finish", runId]);
}

/** Opens the Runs screen through the menu and waits for `runId`'s row. */
async function openRuns(page: Page, runId: string): Promise<void> {
  await page
    .getByRole("navigation", { name: "Console" })
    .getByRole("link", { name: "Runs" })
    .click();
  await expect(page.getByRole("rowheader", { name: runId })).toBeVisible();
}

test(
  "run summaries stay readable after the Daemon stops, moving between screens with the browser offline",
  { annotation: { type: "covers", description: "story:12-126" } },
  async ({ page, context, daemon, signIn, failOnErrors }) => {
    await finishedRun(daemon, "kept-run");
    await signIn.withLink(page);
    await openRuns(page, "kept-run");
    await page.getByRole("link", { name: "kept-run" }).click();
    await expect(page.getByRole("definition").first()).toBeVisible();
    await expect
      .poll(async () => (await stored(page)).kinds.toSorted())
      .toEqual(["run_page", "run_summary"]);
    await openRuns(page, "kept-run");

    await command(daemon, ["daemon", "stop"]);
    await context.setOffline(true);
    await expect(page.getByRole("alert")).toHaveText(BANNER);

    await page.getByRole("link", { name: "kept-run" }).click();
    await expect(page.getByRole("heading", { level: 1 })).toHaveText("Run kept-run");
    await expect(page.getByRole("definition").first()).toHaveText("finished");
    await page
      .getByRole("navigation", { name: "Console" })
      .getByRole("link", { name: "Runs" })
      .click();
    await expect(page.getByText("This browser's offline copy", { exact: false })).toBeVisible();
    await expect(page.getByRole("rowheader", { name: "kept-run" })).toBeVisible();
    expect((await stored(page)).text).not.toContain("kept-run");
    // Away from the Console, so it stops trying to reconnect; each attempt
    // while the Daemon was stopped failed, as the browser says.
    await page.goto("about:blank");
    expect(
      failOnErrors.errors.every(
        ({ text, url }) =>
          /\/protocol|ERR_CONNECTION_REFUSED|ERR_INTERNET_DISCONNECTED|ERR_FAILED/.test(text) ||
          url.includes("/protocol"),
      ),
    ).toBe(true);
    await failOnErrors.take();
  },
);

test(
  "Sign out wipes the offline cache",
  { annotation: { type: "covers", description: "story:12-126" } },
  async ({ page, daemon, signIn, failOnErrors }) => {
    await finishedRun(daemon, "kept-run");
    await signIn.withLink(page);
    await openRuns(page, "kept-run");
    await expect.poll(async () => (await stored(page)).kinds).toEqual(["run_summary"]);

    await page.getByRole("button", { name: "Sign out" }).click();

    await expect(page.getByRole("status")).toHaveText(/^Not signed in/);
    expect((await stored(page)).exists).toBe(false);
    // The page signed out asks `GET /session`, which answers 401, as the browser logs.
    const refused = await failOnErrors.take();
    expect(refused.length).toBeGreaterThan(0);
    expect(refused.every(({ text }) => /401/.test(text))).toBe(true);
  },
);

test(
  "after a revoked sign-in, the cache's key no longer unwraps, and the page signed out wipes it",
  { annotation: { type: "covers", description: "story:12-126" } },
  async ({ page, daemon, signIn, failOnErrors }) => {
    await finishedRun(daemon, "kept-run");
    await signIn.withLink(page);
    await openRuns(page, "kept-run");
    await expect.poll(async () => (await stored(page)).kinds).toEqual(["run_summary"]);
    expect((await decrypted(page)).rows).toHaveLength(1);
    const before = await decrypted(page);

    // The person revokes the page's own sign-in in the Console, as from another device.
    expect(await revokeOwnSignIn(page, daemon)).toContain('"revoked":true');

    expect(before.status).toBe(200);
    expect((await decrypted(page)).status).toBe(401);
    await page.goto(new URL("/", page.url()).href);
    await expect(page.getByRole("status")).toHaveText(/^Not signed in/);
    expect((await stored(page)).exists).toBe(false);
    // The refused unwrap and the signed-out page's session answered 401, as asked.
    expect(failOnErrors.errors.every(({ text }) => /401/.test(text))).toBe(true);
    await failOnErrors.take();
  },
);

/** The field `name` of `value`, if it is an object that has one. */
function field(value: unknown, name: string): unknown {
  return typeof value === "object" && value !== null
    ? Object.entries(value).find(([key]) => key === name)?.[1]
    : undefined;
}

/** Sets `console.offline_cache` to `false` in the user configuration of the test's home. */
async function turnedOff(daemon: ServedDaemon): Promise<void> {
  let written = false;
  for (const major of await readdir(daemon.home)) {
    const file = join(daemon.home, major, "configuration.toml");
    const text = await readFile(file, "utf8").catch(() => undefined);
    if (text?.includes("console.address") === true) {
      await writeFile(
        file,
        text.replace(
          /console\.address = "[^"]*"/,
          (line) => `${line}\nconsole.offline_cache = false`,
        ),
      );
      written = true;
    }
  }
  expect(written, "the test's home has a user configuration").toBe(true);
}

test(
  "with console.offline_cache set to false, the page writes nothing",
  {
    annotation: [
      { type: "covers", description: "story:12-126" },
      { type: "covers", description: "setting:console.offline_cache" },
    ],
  },
  async ({ page, daemon, signIn }) => {
    await finishedRun(daemon, "kept-run");
    await signIn.withLink(page);
    await page.goto("about:blank");
    await command(daemon, ["daemon", "stop"]);
    await turnedOff(daemon);
    await command(daemon, ["daemon", "start"]);
    const status: unknown = JSON.parse(await command(daemon, ["daemon", "status", "--json"]));
    const url = field(status, "console_url");
    if (typeof url !== "string") {
      throw new Error(`the restarted Daemon serves no Console: ${JSON.stringify(status)}`);
    }
    const asked: string[] = [];
    page.on("request", (request) => asked.push(new URL(request.url()).pathname));

    await page.goto(new URL("/runs", url).href);
    await expect(page.getByRole("rowheader", { name: "kept-run" })).toBeVisible();

    expect((await stored(page)).exists).toBe(false);
    expect(asked).not.toContain("/offline-key");
  },
);

test.describe("Secret canaries", () => {
  test.use({ provingGroundSeed: 906 });

  test(
    "Secret canaries planted in a run's data never reach the offline cache",
    { annotation: { type: "covers", description: "story:12-126" } },
    async ({ page, daemon, signIn }) => {
      const provingGround = daemon.provingGround;
      if (provingGround === undefined) {
        throw new Error("the test's Proving ground did not start");
      }
      const kinds: SecretKind[] = [
        "typed_password",
        "totp_seed",
        "session_cookie",
        "endpoint_token",
      ];
      const canaries: Canary[] = [];
      for (const kind of kinds) {
        const planted: unknown = await (
          await fetch(`http://${provingGround.address}/canaries`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ kind }),
          })
        ).json();
        canaries.push({
          id: String(field(planted, "id")),
          kind,
          value: String(field(planted, "value")),
        });
      }
      await command(daemon, ["run", "start", "--run-id", "secret-run"]);
      await callDaemon(daemon, "run.set_progress_message", {
        run_id: "secret-run",
        message: `signed in with ${canaries.map(({ value }) => value).join(" ")}`,
      });
      await signIn.withLink(page);
      await openRuns(page, "secret-run");
      await page.getByRole("link", { name: "secret-run" }).click();
      await expect(page.getByRole("definition").first()).toBeVisible();
      await expect
        .poll(async () => (await stored(page)).kinds.toSorted())
        .toEqual(["run_page", "run_summary"]);
      await openRuns(page, "secret-run");

      const shown = await page.locator("body").innerText();
      const records = (await stored(page)).text;
      const opened = (await decrypted(page)).rows;

      expect(
        searchCanaries(canaries, [{ name: "the Runs screen", text: shown }]).length,
        "the canaries were planted where the Console shows them",
      ).toBeGreaterThan(0);
      expect(opened).toHaveLength(2);
      expect(
        searchCanaries(canaries, [
          { name: "the cache's records", text: records },
          ...opened.map((text, index) => ({ name: `decrypted row ${index}`, text })),
        ]),
      ).toEqual([]);
    },
  );
});
