// A Dataset view kept offline (spec #593, story 215; ADR 0031), on
// Foxhound's executable built from the commit serving Foxhound's Console
// build, each test with a Daemon and home of its own on port 0. The crawl
// is the Compatibility corpus's crawl head with a Dataset `accounts` added,
// Secret canaries planted in a field the person hides, in each Record's URL
// and in its Provenance. A view kept offline is readable after the Daemon
// stops and the browser goes offline, and its cache holds no canary,
// neither in its records nor once decrypted; once the sign-in is revoked,
// the Daemon unwraps the cache's key no more and the page signed out wipes
// what is left.
import type { DatabaseSync } from "node:sqlite";
import type { Page } from "@playwright/test";
import { searchCanaries, type Canary, type SecretKind } from "@goodfetch/test-support/canaries";
import { command } from "@goodfetch/test-support/daemon-calls";
import {
  decryptedOfflineCache,
  revokeOwnSignIn,
  storedOfflineCache,
} from "@goodfetch/test-support/offline-cache";
import { expect, test, type ServedDaemon } from "@goodfetch/test-support/playwright";
import { CRAWL_RUN, bodyRows, openDatasets } from "./seeded-crawl.ts";

/** The banner of a page with no Connection. */
const BANNER =
  "No connection to the engine: this page is read-only until the Console reconnects, which it does by itself.";

/** What the kept view shows: each Record's number and the field the person kept shown. */
const KEPT_ROWS = [
  ["1", "Shop account"],
  ["2", "Shop account"],
];

/** The field `name` of `value`, if it is an object that has one. */
function field(value: unknown, name: string): unknown {
  return typeof value === "object" && value !== null
    ? Object.entries(value).find(([key]) => key === name)?.[1]
    : undefined;
}

/** Plants a canary of each of `kinds` with the test's Proving ground. */
async function planted(daemon: ServedDaemon, kinds: readonly SecretKind[]): Promise<Canary[]> {
  const provingGround = daemon.provingGround;
  if (provingGround === undefined) {
    throw new Error("the test's Proving ground did not start");
  }
  const canaries: Canary[] = [];
  for (const kind of kinds) {
    const answer: unknown = await (
      await fetch(`http://${provingGround.address}/canaries`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ kind }),
      })
    ).json();
    canaries.push({ id: String(field(answer, "id")), kind, value: String(field(answer, "value")) });
  }
  return canaries;
}

/** Adds the Dataset `accounts`: two Records whose `note`, URL and Provenance hold the canaries `[note, url, provenance]`. */
function withAccounts([note, url, provenance]: readonly Canary[]): (
  database: DatabaseSync,
) => void {
  return (database) => {
    database.prepare("INSERT INTO datasets VALUES ('accounts', 2, NULL, NULL, '[]')").run();
    const insert = database.prepare(
      "INSERT INTO records VALUES ('accounts', ?, '1', ?, '2026-01-05T09:00:00Z', ?, NULL, ?)",
    );
    for (const number of [1, 2]) {
      insert.run(
        number,
        `https://shop.test/account?token=${url?.value ?? ""}`,
        JSON.stringify({ name: "Shop account", note: `signed in with ${note?.value ?? ""}` }),
        JSON.stringify({ request_id: "1", clean_up_rules: provenance?.value ?? "" }),
      );
    }
  };
}

/** Shows `accounts` without its `note` and keeps the view offline. */
async function keepAccounts(page: Page): Promise<void> {
  await page.getByRole("combobox", { name: "Records of the Dataset" }).selectOption("accounts");
  await expect.poll(() => bodyRows(page, "Records of accounts")).toHaveLength(2);
  await page.getByRole("checkbox", { name: "note" }).uncheck();
  await page.getByRole("button", { name: "Apply" }).click();
  await expect.poll(() => bodyRows(page, "Records of accounts")).toEqual(KEPT_ROWS);
  await page.getByRole("button", { name: "Keep offline" }).click();
  await expect(
    page.getByText("Kept offline in this browser: 2 Records, readable until you sign out"),
  ).toBeVisible();
  await expect.poll(async () => (await storedOfflineCache(page)).kinds).toContain("dataset_view");
}

test.use({ product: "foxhound", provingGroundSeed: 1219 });

test(
  "a Dataset view kept offline stays readable after the Daemon stops, with no secret field in the cache",
  { annotation: { type: "covers", description: "story:12-215" } },
  async ({ page, context, daemon, signIn, failOnErrors }) => {
    const canaries = await planted(daemon, ["typed_password", "endpoint_token", "session_cookie"]);
    await openDatasets(page, daemon, signIn, withAccounts(canaries));
    await page.getByRole("combobox", { name: "Records of the Dataset" }).selectOption("accounts");
    await expect.poll(() => bodyRows(page, "Records of accounts")).toHaveLength(2);
    const shown = await page.locator("body").innerText();
    expect(
      searchCanaries(canaries, [{ name: "the Records grid", text: shown }]).length,
      "a canary was planted where the grid shows it",
    ).toBeGreaterThan(0);

    await keepAccounts(page);
    const records = (await storedOfflineCache(page)).text;
    const opened = (await decryptedOfflineCache(page)).rows;
    expect(opened.some((row) => row.includes("Shop account"))).toBe(true);
    expect(
      searchCanaries(canaries, [
        { name: "the cache's records", text: records },
        ...opened.map((text, index) => ({ name: `decrypted row ${String(index)}`, text })),
      ]),
    ).toEqual([]);

    // Moves to Runs while the Daemon runs, then back to the crawl's page
    // without it, as a page whose Daemon stopped.
    await page
      .getByRole("navigation", { name: "Console" })
      .getByRole("link", { name: "Runs" })
      .click();
    await expect(page.getByRole("rowheader", { name: CRAWL_RUN })).toBeVisible();
    await command(daemon, ["daemon", "stop"]);
    await context.setOffline(true);
    await expect(page.getByRole("alert")).toHaveText(BANNER);
    await page.getByRole("link", { name: CRAWL_RUN }).click();
    await page.getByRole("tab", { name: "Datasets", exact: true }).click();

    await expect(page.getByText("This browser's offline copy", { exact: false })).toBeVisible();
    await expect.poll(() => bodyRows(page, "Records of accounts")).toEqual(KEPT_ROWS);
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
  "a Dataset view kept offline is unreadable once its sign-in is revoked, and the page signed out wipes it",
  { annotation: { type: "covers", description: "story:12-215" } },
  async ({ page, daemon, signIn, failOnErrors }) => {
    const canaries = await planted(daemon, ["typed_password", "endpoint_token", "session_cookie"]);
    await openDatasets(page, daemon, signIn, withAccounts(canaries));
    await keepAccounts(page);
    const before = await decryptedOfflineCache(page);
    expect(before.status).toBe(200);
    expect(before.rows.some((row) => row.includes("Shop account"))).toBe(true);

    expect(await revokeOwnSignIn(page, daemon)).toContain('"revoked":true');

    expect((await decryptedOfflineCache(page)).status).toBe(401);
    await page.goto(new URL("/", page.url()).href);
    await expect(page.getByRole("status")).toHaveText(/^Not signed in/);
    expect((await storedOfflineCache(page)).exists).toBe(false);
    // The refused unwrap and the signed-out page's session answered 401, as asked.
    expect(failOnErrors.errors.every(({ text }) => /401/.test(text))).toBe(true);
    await failOnErrors.take();
  },
);
