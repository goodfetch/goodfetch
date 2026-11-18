// A built folder served under a base path on a loopback port 0, so the docs
// site's Playwright pass and lychee read the site as it is built (ADR 0068;
// spec #326, ambiguity 5).
import { createServer } from "node:http";
import sirv from "sirv";

/** A running static server. */
export interface StaticServer {
  /** The base's URL, `http://127.0.0.1:<port><base>`. */
  readonly url: string;
  /** Stops the server, ending its open connections. */
  close(): Promise<void>;
}

/** Serves `folder` under `base` on a loopback port the system chooses. */
export async function serveStatic(folder: string, base: `/${string}/`): Promise<StaticServer> {
  const files = sirv(folder);
  const server = createServer((request, response) => {
    const url = request.url ?? "";
    // A page outside the base is a link the deployed site would break, even
    // when the folder holds it, so it answers 404 like a missing page.
    if (!url.startsWith(base)) {
      response.statusCode = 404;
      response.end();
      return;
    }
    request.url = url.slice(base.length - 1);
    files(request, response);
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (address === null || typeof address === "string") {
    throw new Error(`invariant: a TCP server listens on a port, not ${String(address)}`);
  }
  return {
    url: `http://${address.address}:${address.port}${base}`,
    close: () =>
      new Promise((resolve, reject) => {
        server.close((error) => (error === undefined ? resolve() : reject(error)));
        server.closeAllConnections();
      }),
  };
}
