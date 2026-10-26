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

/** The cache's IndexedDB database (`offline.ts`). */
const DATABASE = "goodfetch-console-offline";

/** The banner of a page with no Connection. */
const BANNER =
  "No connection to the engine: this page is read-only until the Console reconnects, which it does by itself.";

/** Starts the run `runId` and finishes it. */
async function finishedRun(daemon: ServedDaemon, runId: string): Promise<void> {
  await command(daemon, ["run", "start", "--run-id", runId]);
  await command(daemon, ["run", "finish", runId]);
}

/** What the cache's database holds: whether it exists, its rows' kinds, and every record as text, its bytes in base64. */
async function stored(page: Page): Promise<{
  readonly exists: boolean;
  readonly kinds: string[];
  readonly text: string;
}> {
  return page.evaluate(async (name) => {
    const names = (await indexedDB.databases()).map((database) => database.name);
    if (!names.includes(name)) {
      return { exists: false, kinds: [], text: "" };
    }
    const database = await new Promise<IDBDatabase>((resolve, reject) => {
      const opening = indexedDB.open(name);
      opening.addEventListener("success", () => resolve(opening.result));
      opening.addEventListener("error", () => reject(new Error("the cache did not open")));
    });
    const records: unknown[] = [];
    for (const store of database.objectStoreNames) {
      const all = database.transaction(store).objectStore(store).getAll();
      records.push(
        ...(await new Promise<unknown[]>((resolve, reject) => {
          all.addEventListener("success", () => resolve(all.result));
          all.addEventListener("error", () => reject(new Error("the cache did not read")));
        })),
      );
    }
    database.close();
    // oxlint-disable-next-line unicorn/consistent-function-scoping -- it runs in the page, which sees nothing of the test's scope.
    const base64 = (bytes: ArrayBuffer | ArrayBufferView): string =>
      btoa(
        String.fromCharCode(...new Uint8Array(bytes instanceof ArrayBuffer ? bytes : bytes.buffer)),
      );
    const kinds = records.flatMap((record) =>
      typeof record === "object" && record !== null && "kind" in record
        ? [String(record.kind)]
        : [],
    );
    const text = JSON.stringify(records, (_, value: unknown) =>
      value instanceof ArrayBuffer || ArrayBuffer.isView(value) ? base64(value) : value,
    );
    return { exists: true, kinds, text };
  }, DATABASE);
}

/**
 * What the cache's rows hold once decrypted, as the page's own sign-in can
 * read them: the key the stored wrapped form holds, from `POST
 * /offline-key`, opening each row. With the Daemon's refusal, its status.
 */
async function decrypted(
  page: Page,
): Promise<{ readonly status: number; readonly rows: string[] }> {
  return page.evaluate(async (name) => {
    const database = await new Promise<IDBDatabase>((resolve, reject) => {
      const opening = indexedDB.open(name);
      opening.addEventListener("success", () => resolve(opening.result));
      opening.addEventListener("error", () => reject(new Error("the cache did not open")));
    });
    const read = async (store: string): Promise<unknown[]> =>
      new Promise((resolve, reject) => {
        const all = database.transaction(store).objectStore(store).getAll();
        all.addEventListener("success", () => resolve(all.result));
        all.addEventListener("error", () => reject(new Error("the cache did not read")));
      });
    const [keyRecord] = await read("key");
    const rows = await read("rows");
    database.close();
    const signedIn = await fetch("/session");
    if (!signedIn.ok) {
      return { status: signedIn.status, rows: [] };
    }
    const session: unknown = await signedIn.json();
    const csrf =
      typeof session === "object" && session !== null && "csrf_token" in session
        ? String(session.csrf_token)
        : "";
    const wrapped =
      typeof keyRecord === "object" && keyRecord !== null && "wrapped" in keyRecord
        ? String(keyRecord.wrapped)
        : "";
    const answer = await fetch("/offline-key", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf },
      body: JSON.stringify({ wrapped }),
    });
    if (!answer.ok) {
      return { status: answer.status, rows: [] };
    }
    const body: unknown = await answer.json();
    const key = typeof body === "object" && body !== null && "key" in body ? String(body.key) : "";
    const raw = Uint8Array.from(atob(key.replaceAll("-", "+").replaceAll("_", "/")), (c) =>
      c.charCodeAt(0),
    );
    const seal = await crypto.subtle.importKey("raw", raw, "AES-GCM", false, ["decrypt"]);
    const plain: string[] = [];
    for (const row of rows) {
      if (
        typeof row === "object" &&
        row !== null &&
        "iv" in row &&
        "data" in row &&
        "name" in row
      ) {
        const iv = row.iv instanceof Uint8Array ? new Uint8Array(row.iv) : new Uint8Array();
        const data = row.data instanceof ArrayBuffer ? row.data : new ArrayBuffer(0);
        const opened = await crypto.subtle.decrypt(
          { name: "AES-GCM", iv, additionalData: new TextEncoder().encode(String(row.name)) },
          seal,
          data,
        );
        plain.push(new TextDecoder().decode(opened));
      }
    }
    return { status: answer.status, rows: plain };
  }, DATABASE);
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
    const info: unknown = JSON.parse(await command(daemon, ["engine", "info", "--json"]));
    const revoked = await page.evaluate(
      async (productVersion) => {
        const socket = new WebSocket(`ws://${window.location.host}/protocol`);
        const replies = new Map<number, (value: unknown) => void>();
        socket.addEventListener("message", (event) => {
          const message: unknown = JSON.parse(String(event.data));
          if (typeof message === "object" && message !== null && "id" in message) {
            replies.get(Number(message.id))?.(message);
          }
        });
        await new Promise((resolve) => socket.addEventListener("open", resolve));
        const call = async (id: number, method: string, params: unknown): Promise<unknown> =>
          new Promise((resolve) => {
            replies.set(id, resolve);
            socket.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
          });
        const hello: unknown = await call(0, "hello", {
          protocol_version: "0.1",
          surface: "console",
          form: "catalog",
          needs: "collie",
          product_version: productVersion,
        });
        const listed: unknown = await call(1, "sign_in.list", {});
        const text = JSON.stringify(listed);
        const id = /"id":"([0-9a-f]+)"/.exec(text)?.[1] ?? "";
        const answer = JSON.stringify(await call(2, "sign_in.revoke", { id }));
        socket.close();
        return { hello: JSON.stringify(hello), answer };
      },
      String(field(info, "product_version")),
    );
    expect(revoked.answer).toContain('"revoked":true');

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
