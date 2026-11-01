// The Console's content origin, on Collie's executable built from the
// commit, under its production headers, each test with a Daemon and home of
// its own on port 0 (spec #592; ADRs 0031, 0067 and 0068): a Run's file
// arrives by handle through the Client's body fetcher, and is shown only
// from the content origin, by an address signed for it, where the Proving
// ground's page that tries to escape its frame (#167) runs no script, in the
// Console's sandboxed frame or in a frame with no sandbox at all. Its public
// page that calls the local Daemon is refused on the Console's WebSocket and
// endpoints.
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { access, mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { promisify } from "node:util";
import type { BrowserContext } from "@playwright/test";
import { expect, test, type ServedDaemon as Daemon } from "@goodfetch/test-support/playwright";
import { httpBodyFetcher } from "#client/browser.ts";

/** Starts the Run `runId` in the test's home with the executable's `run start`. */
async function startRun(daemon: Daemon, runId: string): Promise<void> {
  await promisify(execFile)(daemon.executable, ["run", "start", "--run-id", runId], {
    env: daemon.env,
  });
}

/**
 * Stores `bytes` as a body of the Run `runId`, as the engine does: a file of
 * its folder's `bodies/`, named by their lowercase hex SHA-256. Answers the
 * body's handle, `<run_id>/<body>`.
 */
async function storeBody(daemon: Daemon, runId: string, bytes: Uint8Array): Promise<string> {
  const id = createHash("sha256").update(bytes).digest("hex");
  for (const major of await readdir(daemon.home)) {
    const folder = join(daemon.home, major, "runs", runId);
    const exists = await access(folder).then(
      () => true,
      () => false,
    );
    if (exists) {
      await mkdir(join(folder, "bodies"), { recursive: true });
      await writeFile(join(folder, "bodies", id), bytes);
      return `${runId}/${id}`;
    }
  }
  throw new Error(`the test's home holds no Run ${runId}`);
}

/** The `Cookie` header the context sends to the Console. */
async function cookieHeader(context: BrowserContext, url: string): Promise<string> {
  const cookies = await context.cookies(url);
  return cookies.map((cookie) => `${cookie.name}=${cookie.value}`).join("; ");
}

/** The text of every log file the test's Goodfetch home holds. */
async function logsOf(home: string): Promise<string> {
  const folder = join(home, "logs");
  const names = await readdir(folder);
  const texts = await Promise.all(names.map(async (name) => readFile(join(folder, name), "utf8")));
  return texts.join("\n");
}

test(
  "a Run's page body arrives by handle through the Client's body fetcher, to a sign-in only",
  { annotation: { type: "covers", description: "story:12-127" } },
  async ({ page, context, daemon, signIn }) => {
    await signIn.withLink(page);
    await startRun(daemon, "first");
    const bytes = new TextEncoder().encode("<!doctype html><title>A body</title><p>A body.</p>");
    const handle = await storeBody(daemon, "first", bytes);
    const origin = new URL(daemon.url).origin;
    const cookie = await cookieHeader(context, daemon.url);
    const signedIn: typeof fetch = async (input, init) =>
      fetch(input, { ...init, headers: { Cookie: cookie } });

    const fetched = await httpBodyFetcher(origin, signedIn)(handle);

    expect(fetched).toEqual(bytes);
    await expect(httpBodyFetcher(origin, fetch)(handle)).rejects.toThrow(/answered 401/);
  },
);

test.describe("with the Proving ground", () => {
  test.use({ provingGroundSeed: 896 });

  test(
    "the escape page shown on the content origin runs no script, in a sandboxed frame or any",
    {
      annotation: [
        { type: "covers", description: "story:12-127" },
        { type: "covers", description: "story:12-129" },
      ],
    },
    async ({ page, context, daemon, signIn, failOnErrors }) => {
      const provingGround = daemon.provingGround;
      if (provingGround === undefined) {
        throw new Error("invariant: the test asked for a Proving ground");
      }
      const port = provingGround.address.split(":").at(-1) ?? "";
      const escape = `http://adversarial.localhost:${port}/escape`;
      const escapes: string[] = [];
      context.on("request", (request) => {
        const url = new URL(request.url());
        if (url.pathname === "/reports" || url.pathname === "/escape/landed") {
          escapes.push(request.url());
        }
      });

      // The control: framed by a page of its own origin with no sandbox,
      // the page's script runs and escapes through a popup.
      const control = await context.newPage();
      const popup = context.waitForEvent("page");
      await control.goto(`http://adversarial.localhost:${port}/injection`);
      await control.evaluate((url) => {
        const frame = document.createElement("iframe");
        frame.title = "The escape page";
        frame.src = url;
        document.body.append(frame);
      }, escape);
      const landed = await popup;
      expect(new URL(landed.url()).searchParams.get("attempt")).toBe("popup");
      await expect.poll(() => escapes.length).toBeGreaterThan(0);
      await landed.close();
      await control.close();
      escapes.length = 0;
      await failOnErrors.take();

      // The same page, stored as a Run's body and shown from its address on
      // the content origin.
      await signIn.withLink(page);
      await startRun(daemon, "first");
      const html = await (await fetch(escape)).arrayBuffer();
      const handle = await storeBody(daemon, "first", new Uint8Array(html));
      const address = await page.evaluate(async (path) => {
        const response = await fetch(`/content-address/${path}?type=html`);
        const answer: unknown = await response.json();
        if (typeof answer !== "object" || answer === null || !("url" in answer)) {
          throw new Error(`the Console answered no content address: ${JSON.stringify(answer)}`);
        }
        return String(answer.url);
      }, handle);
      expect(new URL(address).origin).not.toBe(new URL(daemon.url).origin);
      const pages = context.pages().length;
      await page.evaluate((url) => {
        for (const [title, sandbox] of [
          ["The scraped page, sandboxed", ""],
          ["The scraped page, with no sandbox", undefined],
        ] as const) {
          const frame = document.createElement("iframe");
          frame.title = title;
          if (sandbox !== undefined) {
            frame.setAttribute("sandbox", sandbox);
          }
          frame.src = url;
          document.body.append(frame);
        }
      }, address);

      const shown = () => page.frames().filter((frame) => frame.url() === address);
      await expect.poll(() => shown().length).toBe(2);
      for (const frame of shown()) {
        // The page's HTML renders.
        await expect(frame.locator("main")).toBeVisible();
      }
      // Each frame's script was refused, the sandbox of the content origin's
      // policy holding where the frame asked for none: the browser says so
      // on the Console's page, naming the address, once for each.
      const blocked = `Blocked script execution in '${address}' because the document's frame is sandboxed`;
      await expect
        .poll(() => failOnErrors.errors.filter((error) => error.text.startsWith(blocked)).length)
        .toBeGreaterThanOrEqual(2);
      const refused = await failOnErrors.take();
      expect(refused.filter((error) => !error.text.startsWith(blocked))).toEqual([]);
      expect(escapes).toEqual([]);
      expect(context.pages().length).toBe(pages);
      await page.evaluate(() => {
        for (const frame of document.querySelectorAll("iframe")) {
          frame.remove();
        }
      });
    },
  );

  test(
    "the public page that calls the local Daemon is refused on the Console's WebSocket and endpoints",
    { annotation: { type: "covers", description: "story:12-129" } },
    async ({ page, context, daemon, signIn, failOnErrors }) => {
      const provingGround = daemon.provingGround;
      if (provingGround === undefined) {
        throw new Error("invariant: the test asked for a Proving ground");
      }
      await signIn.withLink(page);
      const port = provingGround.address.split(":").at(-1) ?? "";
      const reports: string[] = [];
      context.on("request", (request) => {
        if (new URL(request.url()).pathname === "/reports") {
          reports.push(request.url());
        }
      });
      const caller = await context.newPage();
      const sockets: Promise<string>[] = [];
      caller.on("websocket", (socket) => {
        sockets.push(
          new Promise((settled) => {
            socket.on("close", () => settled(socket.url()));
            socket.on("socketerror", () => settled(socket.url()));
          }),
        );
      });

      await caller.goto(
        `http://adversarial.localhost:${port}/daemon?daemon=127.0.0.1:${daemon.port}`,
      );
      await expect.poll(() => sockets.length).toBe(2);
      const ended = await Promise.all(sockets);

      expect(ended.map((url) => new URL(url).pathname).toSorted()).toEqual(["/", "/protocol"]);
      // The Console's WebSocket refused the page's `Origin`, and logged it.
      await expect.poll(async () => logsOf(daemon.home)).toContain("origin_foreign");
      await caller.close();
      await failOnErrors.take();
      expect(reports).toEqual([]);
    },
  );
});
