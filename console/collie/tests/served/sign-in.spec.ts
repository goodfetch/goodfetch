// Opening Collie's Console signed in (spec #592; ADRs 0024, 0031 and 0068),
// on Collie's executable built from the commit, under its production
// headers, each test with a Daemon and home of its own on port 0: the link
// `console` gives carries the Daemon's token in its fragment, which the
// page swaps once for an `HttpOnly` cookie and takes out of the address;
// the Console then reaches the engine over its WebSocket and shows its
// Product and version. A request with no cookie, or a wrong token, is
// refused, and the token is in no log and no `Referer`.
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test } from "@goodfetch/test-support/playwright";

/** The text of every log file the test's Goodfetch home holds. */
async function logsOf(home: string): Promise<string[]> {
  const folder = join(home, "logs");
  const names = await readdir(folder);
  return Promise.all(names.map(async (name) => readFile(join(folder, name), "utf8")));
}

test(
  "the sign-in link swaps its token for an HttpOnly cookie, then the Console shows its engine",
  { annotation: { type: "covers", description: "story:12-1" } },
  async ({ page, context, daemon, signIn }) => {
    const requests: { readonly url: string; readonly referer: string | undefined }[] = [];
    const sockets: string[] = [];
    page.on("request", (request) => {
      requests.push({ url: request.url(), referer: request.headers()["referer"] });
    });
    page.on("websocket", (socket) => sockets.push(socket.url()));

    await signIn.withLink(page);

    await expect(page.getByRole("main").getByRole("status")).toHaveText(
      /^Connected to Collie \d+\.\d+\.\d+\S*\.$/,
    );
    expect(new URL(page.url()).hash).toBe("");
    const cookies = await context.cookies(daemon.url);
    expect(cookies).toEqual([
      expect.objectContaining({ httpOnly: true, sameSite: "Strict", path: "/" }),
    ]);
    expect(cookies[0]?.value).not.toBe(daemon.token);
    expect(requests.map((request) => new URL(request.url).pathname)).toEqual(
      expect.arrayContaining(["/", "/sign-in", "/session"]),
    );
    for (const request of requests) {
      expect(request.url).not.toContain(daemon.token);
      expect(request.referer ?? "").not.toContain(daemon.token);
    }
    expect(sockets.map((socket) => new URL(socket).pathname)).toEqual(["/protocol"]);
    expect(sockets[0]).not.toContain(daemon.token);
  },
);

test(
  "a request with no cookie or a wrong token is refused, and the token is in no log",
  { annotation: { type: "covers", description: "story:12-1" } },
  async ({ page, daemon, signIn }) => {
    await signIn.withLink(page);
    const origin = new URL(daemon.url).origin;
    const wrong = daemon.token.replace(/./g, "0");

    // Outside the browser, so the request carries no cookie of the page's.
    const unsigned = await fetch(daemon.session);
    const refused = await fetch(new URL("/sign-in", origin), {
      method: "POST",
      headers: { Origin: origin, "Content-Type": "text/plain;charset=UTF-8" },
      body: wrong,
    });

    expect(unsigned.status).toBe(401);
    expect(refused.status).toBe(401);
    expect(refused.headers.get("set-cookie")).toBeNull();
    const logs = await logsOf(daemon.home);
    expect(logs.length).toBeGreaterThan(0);
    for (const log of logs) {
      expect(log).not.toContain(daemon.token);
    }
  },
);
