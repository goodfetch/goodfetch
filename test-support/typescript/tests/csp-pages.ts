// A local server of the fixture pages in `fixtures/csp-pages/`, each under
// the production CSP of ADR 0031: the page's own scripts only, no inline code
// or `eval`, Trusted Types required for every script sink, and violations
// reported to the page's own origin. It also answers a same-origin WebSocket
// at `/echo` and drops the connection of `/broken`, for the Playwright tests
// of test support's fixtures.
import { readFile } from "node:fs/promises";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { extname, join } from "node:path";
import { text } from "node:stream/consumers";
import { WebSocketServer } from "ws";

/**
 * The strict Content Security Policy served tests run under (ADR 0068).
 * A page given `contentOrigin` may frame that origin and nothing else's, and loosens no other directive.
 */
export function productionCsp(contentOrigin?: string): string {
  return [
    "default-src 'none'",
    "script-src 'self'",
    "style-src 'self'",
    "img-src 'self'",
    "font-src 'self'",
    "connect-src 'self'",
    "manifest-src 'self'",
    ...(contentOrigin === undefined ? [] : [`frame-src ${contentOrigin}`]),
    "base-uri 'none'",
    "form-action 'self'",
    "frame-ancestors 'none'",
    "require-trusted-types-for 'script'",
    "trusted-types 'none'",
    "report-uri /csp-report",
  ].join("; ");
}

/** The policy with no framed origin, which the fixture pages serve. */
export const PRODUCTION_CSP: string = productionCsp();

/** The served fixture pages. */
export interface CspPages {
  /** The server's origin, `http://127.0.0.1:<port>`. */
  readonly origin: string;
  /** The URL of a fixture page, by its file name without `.html`. */
  url(page: string): string;
  /** Each CSP report the pages sent, as its body. */
  readonly reports: readonly string[];
  close(): Promise<void>;
}

const FOLDER = join(import.meta.dirname, "fixtures", "csp-pages");

const CONTENT_TYPES: Readonly<Record<string, string>> = {
  ".css": "text/css; charset=utf-8",
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".png": "image/png",
};

/** Serves the fixture pages on port 0 of the loopback address. */
export async function serveCspPages(): Promise<CspPages> {
  const reports: string[] = [];
  const server = createServer((request, response) => {
    void answer(request, response, reports);
  });
  const sockets = new WebSocketServer({ noServer: true });
  sockets.on("connection", (socket) => {
    socket.on("message", (data, isBinary) => {
      socket.send(`echo:${isBinary || !Buffer.isBuffer(data) ? "binary" : data.toString("utf8")}`);
    });
  });
  server.on("upgrade", (request, socket, head) => {
    if (request.url !== "/echo") {
      socket.destroy();
      return;
    }
    sockets.handleUpgrade(request, socket, head, (client) => {
      sockets.emit("connection", client, request);
    });
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (address === null || typeof address === "string") {
    throw new Error("the fixture server listens on no TCP port");
  }
  const origin = `http://127.0.0.1:${address.port}`;
  return {
    origin,
    url: (page) => `${origin}/${page}.html`,
    reports,
    close: async () => {
      for (const client of sockets.clients) {
        client.terminate();
      }
      sockets.close();
      server.closeAllConnections();
      await new Promise<void>((resolve, reject) => {
        server.close((error) => (error === undefined ? resolve() : reject(error)));
      });
    },
  };
}

/** The worker-scoped fixture of a spec file that serves the pages: `test.extend<object, { cspPages: CspPages }>({ cspPages: cspPagesFixture })`. */
export const cspPagesFixture: [
  (fixtures: object, use: (pages: CspPages) => Promise<void>) => Promise<void>,
  { scope: "worker" },
] = [
  // oxlint-disable-next-line no-empty-pattern -- Playwright reads a fixture's dependencies from its first parameter, which must be a destructuring pattern.
  async ({}, use) => {
    const pages = await serveCspPages();
    await use(pages);
    await pages.close();
  },
  { scope: "worker" },
];

async function answer(
  request: IncomingMessage,
  response: ServerResponse,
  reports: string[],
): Promise<void> {
  const path = new URL(request.url ?? "/", "http://fixture").pathname;
  response.setHeader("Content-Security-Policy", PRODUCTION_CSP);
  if (path === "/csp-report" && request.method === "POST") {
    reports.push(await text(request));
    response.writeHead(204).end();
    return;
  }
  if (path === "/broken") {
    request.socket.destroy();
    return;
  }
  const file = await fixtureFile(path);
  if (file === undefined) {
    response.writeHead(404).end();
    return;
  }
  response.writeHead(200, { "Content-Type": file.type }).end(file.body);
}

/** The fixture page or page file at `path` (`/clean.html`, `/clean.js`), with its content type; `undefined` when there is none. */
export async function fixtureFile(
  path: string,
): Promise<{ readonly type: string; readonly body: Buffer } | undefined> {
  const type = CONTENT_TYPES[extname(path)];
  const name = path.slice(1);
  if (type === undefined || name.includes("/")) {
    return undefined;
  }
  try {
    return { type, body: await readFile(join(FOLDER, name)) };
  } catch {
    return undefined;
  }
}
