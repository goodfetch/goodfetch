// What tells one Daemon's own Console listener apart from anything else on
// its port (ADRs 0031 and 0068). The Daemon binds its Console on port 0, so
// once it has stopped, another process may bind the freed port: a connect
// that succeeds proves only that something listens there. A sign-in proves
// more: only the Daemon whose accounts store made it answers its cookie at
// `GET /session`, with the CSRF token it gave it. Another Daemon's Console
// answers `401`, and any other listener something else.
import { TestSupportError } from "./errors.ts";
import type { ServedDaemon } from "./served-daemon.ts";

/** A sign-in on one Daemon's Console, which only that Daemon's listener answers as its own. */
export interface ConsoleIdentity {
  /** The Console's session address, `http://127.0.0.1:<port>/session`. */
  readonly session: string;
  /** The sign-in's cookie, as a `Cookie` header's `name=value` pair. */
  readonly cookie: string;
  /** The CSRF token the Console gave the sign-in. */
  readonly csrfToken: string;
}

/**
 * How long a listener has to answer `GET /session`: a Daemon's Console
 * answers it from its accounts store in milliseconds, so a listener that
 * accepts and says nothing for this long is another program.
 */
const ANSWER_BOUND_MS = 10_000;

/** The CSRF token `GET /session`'s body names, if any. */
function csrfTokenOf(body: unknown): string | undefined {
  if (typeof body === "object" && body !== null && "csrf_token" in body) {
    const token = body.csrf_token;
    return typeof token === "string" ? token : undefined;
  }
  return undefined;
}

/**
 * Each request closes its connection once answered, so no idle connection
 * outlives it into the Daemon's stop.
 */
const CLOSING = { connection: "close" } as const;

/** The CSRF token the listener at `session` answers `cookie` with, if it answers one. */
async function sessionToken(session: string, cookie: string): Promise<string | undefined> {
  const answer = await fetch(session, {
    headers: { ...CLOSING, cookie },
    redirect: "manual",
    signal: AbortSignal.timeout(ANSWER_BOUND_MS),
  });
  if (answer.status !== 200) {
    return undefined;
  }
  try {
    return csrfTokenOf(await answer.json());
  } catch {
    return undefined;
  }
}

/**
 * Signs in to the Daemon's own Console through the executable's sign-in
 * link, as the Console page does: the link's token, which its fragment
 * carries, swapped at `POST /sign-in` for the sign-in's cookie; fails with
 * `sign_in_failed` when the Console refuses it.
 */
export async function consoleIdentity(
  daemon: Pick<ServedDaemon, "session" | "signInLink">,
): Promise<ConsoleIdentity> {
  const link = new URL(await daemon.signInLink());
  const token = new URLSearchParams(link.hash.replace(/^#/, "")).get("token");
  if (token === null) {
    throw new TestSupportError(
      "sign_in_failed",
      `the sign-in link carries no token: ${link.origin}${link.pathname}`,
    );
  }
  const signedIn = await fetch(new URL("/sign-in", link.origin), {
    method: "POST",
    body: token,
    // The Console takes a sign-in only from its own origin, as its page sends it.
    headers: { ...CLOSING, "content-type": "text/plain;charset=UTF-8", origin: link.origin },
    signal: AbortSignal.timeout(ANSWER_BOUND_MS),
  });
  const cookie = signedIn.headers.getSetCookie()[0]?.split(";")[0];
  if (signedIn.status !== 204 || cookie === undefined) {
    throw new TestSupportError(
      "sign_in_failed",
      `\`POST /sign-in\` answered ${String(signedIn.status)}${cookie === undefined ? " with no cookie" : ""}`,
    );
  }
  const csrfToken = await sessionToken(daemon.session, cookie);
  if (csrfToken === undefined) {
    throw new TestSupportError(
      "sign_in_failed",
      "`GET /session` did not answer the new sign-in's cookie",
    );
  }
  return { session: daemon.session, cookie, csrfToken };
}

/**
 * Whether the Console that gave `identity` still listens on its port: the
 * listener there answers the sign-in's cookie with its CSRF token. A
 * refused connect, any other answer, or none in time, is another
 * listener or none.
 */
export async function answersAs(identity: ConsoleIdentity): Promise<boolean> {
  try {
    return (await sessionToken(identity.session, identity.cookie)) === identity.csrfToken;
  } catch {
    return false;
  }
}
