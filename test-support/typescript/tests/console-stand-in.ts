// A stand-in for the Console a Product's executable serves, until area 12's
// Console is built: the `consoleServer` the tests of test support's `daemon`
// and `signIn` fixtures pass. It serves, on port 0 of the loopback address
// and under the production CSP:
//
// - `/`: the Daemon it stands beside, by the Product that answered a
//   `hello` with the Daemon's token, and who is signed in;
// - `/sign-in/<token>`: the single-user sign-in link, which sets the
//   session cookie (`HttpOnly`, `SameSite=Strict`) and goes back to `/`;
//   any other token shows that the link expired and signs nobody in;
// - `/sso` and `/sso/callback`: single sign-on through the test's OIDC
//   provider, with the authorization code flow and PKCE;
// - `/session`: 204 for a signed-in request, 401 otherwise;
// - `/passkey/*`: passkey registration and step-up for a signed-in person,
//   which checks each ceremony's challenge, type and origin.
import { createHash, randomBytes } from "node:crypto";
import { readFile } from "node:fs/promises";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { join } from "node:path";
import { text } from "node:stream/consumers";
import type { Product } from "@goodfetch/test-support";
import type {
  ConsoleServer,
  ConsoleStart,
  ServedConsole,
} from "@goodfetch/test-support/playwright";
import { PRODUCTION_CSP } from "./csp-pages.ts";
import { hello } from "./daemon-hello.ts";

const SESSION_COOKIE = "goodfetch_session";
const SCRIPT = join(import.meta.dirname, "fixtures", "console-stand-in", "console.js");
const PRODUCT_NAMES = { collie: "Collie", foxhound: "Foxhound" } as const satisfies Record<
  Product,
  string
>;

/** How a stand-in Console behaves. */
export interface StandInOptions {
  /** Gives a sign-in link that has already expired. */
  readonly expiredLinks?: boolean;
}

/** One person's session, by the cookie's value. */
interface Session {
  readonly person: string;
  challenge: string | undefined;
  passkey: string | undefined;
}

/** A single sign-on in progress, by its `state`. */
interface SingleSignOn {
  readonly verifier: string;
}

/** The `consoleServer` of a test: a stand-in Console beside the test's Daemon. */
export function standInConsole(options: StandInOptions = {}): ConsoleServer {
  return { serve: (start) => serveStandIn(start, options) };
}

async function serveStandIn(start: ConsoleStart, options: StandInOptions): Promise<ServedConsole> {
  const links = new Set<string>();
  const sessions = new Map<string, Session>();
  const signOns = new Map<string, SingleSignOn>();
  let origin = "";
  const server = createServer((request, response) => {
    response.setHeader("Content-Security-Policy", PRODUCTION_CSP);
    const context = { start, origin, links, sessions, signOns, request, response };
    answer(context).catch((error: unknown) => {
      response.writeHead(500, { "Content-Type": "text/plain" }).end(String(error));
    });
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (address === null || typeof address === "string") {
    throw new Error("the stand-in Console listens on no TCP port");
  }
  origin = `http://127.0.0.1:${address.port}`;
  return {
    url: `${origin}/`,
    singleSignOn: `${origin}/sso`,
    session: `${origin}/session`,
    signInLink: () => {
      const token = randomBytes(16).toString("base64url");
      if (options.expiredLinks !== true) {
        links.add(token);
      }
      return Promise.resolve(`${origin}/sign-in/${token}`);
    },
    close: async () => {
      server.closeAllConnections();
      await new Promise<void>((resolve, reject) => {
        server.close((error) => (error === undefined ? resolve() : reject(error)));
      });
    },
  };
}

interface Exchange {
  readonly start: ConsoleStart;
  readonly origin: string;
  readonly links: Set<string>;
  readonly sessions: Map<string, Session>;
  readonly signOns: Map<string, SingleSignOn>;
  readonly request: IncomingMessage;
  readonly response: ServerResponse;
}

async function answer(exchange: Exchange): Promise<void> {
  const { request, response, start, origin } = exchange;
  const url = new URL(request.url ?? "/", origin);
  const session = sessionOf(exchange);
  if (url.pathname === "/") {
    await home(exchange, session);
  } else if (url.pathname === "/console.js") {
    response
      .writeHead(200, { "Content-Type": "text/javascript; charset=utf-8" })
      .end(await readFile(SCRIPT));
  } else if (url.pathname.startsWith("/sign-in/")) {
    const token = url.pathname.slice("/sign-in/".length);
    if (!exchange.links.delete(token)) {
      page(response, "Sign-in link expired", "<p>This sign-in link has expired.</p>");
      return;
    }
    signedIn(exchange, process.env["USER"] ?? "owner");
  } else if (url.pathname === "/sso") {
    const state = randomBytes(16).toString("base64url");
    const verifier = randomBytes(32).toString("base64url");
    exchange.signOns.set(state, { verifier });
    const authorize = new URL(`${start.oidc.issuer}/auth`);
    authorize.search = new URLSearchParams({
      client_id: start.oidc.clientId,
      response_type: "code",
      scope: "openid profile",
      redirect_uri: `${origin}${start.oidc.redirectPath}`,
      state,
      code_challenge: createHash("sha256").update(verifier).digest("base64url"),
      code_challenge_method: "S256",
    }).toString();
    response.writeHead(303, { Location: authorize.href }).end();
  } else if (url.pathname === start.oidc.redirectPath) {
    await singleSignOnCallback(exchange, url);
  } else if (url.pathname === "/session") {
    response.writeHead(session === undefined ? 401 : 204).end();
  } else if (url.pathname === "/passkey/challenge" && session !== undefined) {
    session.challenge = randomBytes(32).toString("base64url");
    json(response, { challenge: session.challenge, person: session.person });
  } else if (url.pathname === "/passkey/register" && session !== undefined) {
    const ceremony = await ceremonyOf(exchange, session, "webauthn.create");
    if (ceremony !== undefined) {
      session.passkey = ceremony.id;
    }
    json(response, { done: ceremony !== undefined });
  } else if (url.pathname === "/passkey/step-up" && session !== undefined) {
    const ceremony = await ceremonyOf(exchange, session, "webauthn.get");
    json(response, { done: ceremony !== undefined && ceremony.id === session.passkey });
  } else {
    response.writeHead(404).end();
  }
}

async function home(exchange: Exchange, session: Session | undefined): Promise<void> {
  const { daemon, product } = exchange.start;
  const answered = await hello(daemon, daemon.token, product);
  const answeredProduct = stringOf(field(field(answered, "result"), "product"));
  const name =
    answeredProduct === "collie" || answeredProduct === "foxhound"
      ? `${PRODUCT_NAMES[answeredProduct]}'s Daemon`
      : "No Daemon";
  const status =
    session === undefined
      ? '<p id="status">Signed out</p><p><a href="/sso">Sign in with single sign-on</a></p>'
      : `<p id="status">Signed in as ${session.person}</p>
        <button id="register" type="button">Register a passkey</button>
        <button id="step-up" type="button">Confirm with your passkey</button>
        <p id="passkey" role="status">No passkey</p>
        <script src="/console.js"></script>`;
  page(exchange.response, name, status);
}

async function singleSignOnCallback(exchange: Exchange, url: URL): Promise<void> {
  const { start, origin, response } = exchange;
  const state = url.searchParams.get("state") ?? "";
  const signOn = exchange.signOns.get(state);
  exchange.signOns.delete(state);
  const code = url.searchParams.get("code");
  if (signOn === undefined || code === null) {
    page(response, "Single sign-on failed", "<p>The provider did not sign you in.</p>");
    return;
  }
  const discovery: unknown = await (
    await fetch(`${start.oidc.issuer}/.well-known/openid-configuration`)
  ).json();
  const credentials = Buffer.from(`${start.oidc.clientId}:${start.oidc.clientSecret}`).toString(
    "base64",
  );
  const tokens = await fetch(stringOf(field(discovery, "token_endpoint")) ?? "", {
    method: "POST",
    headers: { Authorization: `Basic ${credentials}` },
    body: new URLSearchParams({
      grant_type: "authorization_code",
      code,
      redirect_uri: `${origin}${start.oidc.redirectPath}`,
      code_verifier: signOn.verifier,
    }),
  });
  const accessToken = stringOf(field(await tokens.json(), "access_token"));
  const userinfo = await fetch(stringOf(field(discovery, "userinfo_endpoint")) ?? "", {
    headers: { Authorization: `Bearer ${accessToken ?? ""}` },
  });
  const sub = userinfo.ok ? stringOf(field(await userinfo.json(), "sub")) : undefined;
  if (sub === undefined) {
    page(response, "Single sign-on failed", "<p>The provider did not sign you in.</p>");
    return;
  }
  signedIn(exchange, sub);
}

/** The ceremony a passkey request carries, when its challenge, type and origin are the expected ones. */
async function ceremonyOf(
  exchange: Exchange,
  session: Session,
  type: "webauthn.create" | "webauthn.get",
): Promise<{ id: string } | undefined> {
  const body: unknown = JSON.parse(await text(exchange.request));
  const id = stringOf(field(body, "id"));
  const encoded = stringOf(field(body, "clientData")) ?? "";
  const clientData: unknown = JSON.parse(Buffer.from(encoded, "base64url").toString("utf8"));
  const expected = session.challenge;
  session.challenge = undefined;
  return id !== undefined &&
    field(clientData, "type") === type &&
    field(clientData, "challenge") === expected &&
    field(clientData, "origin") === exchange.origin
    ? { id }
    : undefined;
}

/** A field of a parsed JSON value; `undefined` when the value is no object or lacks it. */
function field(value: unknown, key: string): unknown {
  return typeof value === "object" && value !== null ? Reflect.get(value, key) : undefined;
}

/** A parsed JSON value when it is a string. */
function stringOf(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function sessionOf(exchange: Exchange): Session | undefined {
  const cookies = exchange.request.headers.cookie ?? "";
  for (const cookie of cookies.split(";")) {
    const [name, value] = cookie.trim().split("=");
    if (name === SESSION_COOKIE && value !== undefined) {
      return exchange.sessions.get(value);
    }
  }
  return undefined;
}

function signedIn(exchange: Exchange, person: string): void {
  const value = randomBytes(32).toString("base64url");
  exchange.sessions.set(value, { person, challenge: undefined, passkey: undefined });
  exchange.response
    .writeHead(303, {
      Location: "/",
      "Set-Cookie": `${SESSION_COOKIE}=${value}; HttpOnly; SameSite=Strict; Path=/`,
    })
    .end();
}

function page(response: ServerResponse, title: string, body: string): void {
  response
    .writeHead(200, { "Content-Type": "text/html; charset=utf-8" })
    .end(document(title, body));
}

function document(title: string, body: string): string {
  return `<!doctype html>
<html lang="en">
  <head><meta charset="utf-8" /><title>${title}</title></head>
  <body><main><h1>${title}</h1>${body}</main></body>
</html>`;
}

function json(response: ServerResponse, value: unknown): void {
  response.writeHead(200, { "Content-Type": "application/json" }).end(JSON.stringify(value));
}
