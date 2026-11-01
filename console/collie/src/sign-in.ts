// Signing in on a person's own machine (ADRs 0024 and 0031): the sign-in
// link carries the Daemon's token in its URL fragment, which no request
// ever sends; the page swaps it once for the Console's `HttpOnly` cookie at
// `POST /sign-in`, then takes it out of the address, so neither the
// address, the history nor a `Referer` keeps it. `GET /session` then says
// whether the page is signed in, with the sign-in's CSRF token and whether
// `console.offline_cache` lets it cache run summaries; `POST /sign-out`
// ends the sign-in.

/** The parts of `window` signing in reads and changes. */
export interface SignInPage {
  readonly location: Pick<Location, "hash" | "pathname" | "search" | "origin">;
  readonly history: Pick<History, "replaceState" | "state">;
  readonly fetch: typeof fetch;
}

/** The token a sign-in link's fragment carries, if any. */
function tokenOf(hash: string): string | undefined {
  return new URLSearchParams(hash.replace(/^#/, "")).get("token") ?? undefined;
}

/** A page signed in: the sign-in's CSRF token, which every `POST` of the page carries, and whether it may cache run summaries. */
export interface SignedIn {
  readonly csrfToken: string;
  readonly offlineCache: boolean;
}

/** Whether `GET /session`'s body lets the page cache run summaries. */
function offlineCacheOf(body: unknown): boolean {
  return typeof body === "object" && body !== null && "offline_cache" in body
    ? body.offline_cache === true
    : false;
}

/** The CSRF token `GET /session`'s body names, if any. */
function csrfTokenOf(body: unknown): string | undefined {
  if (typeof body === "object" && body !== null && "csrf_token" in body) {
    const token = body.csrf_token;
    return typeof token === "string" ? token : undefined;
  }
  return undefined;
}

/**
 * Swaps the sign-in link's token, when the address carries one, for the
 * Console's cookie, takes it out of the address, and answers the page's
 * sign-in, if it is signed in.
 */
export async function signIn(page: SignInPage): Promise<SignedIn | undefined> {
  const { location, history } = page;
  const token = tokenOf(location.hash);
  if (token !== undefined) {
    try {
      await page.fetch(new URL("/sign-in", location.origin), {
        method: "POST",
        body: token,
        credentials: "same-origin",
        headers: { "Content-Type": "text/plain;charset=UTF-8" },
      });
    } finally {
      history.replaceState(history.state, "", `${location.pathname}${location.search}`);
    }
  }
  const session = await page.fetch(new URL("/session", location.origin), {
    credentials: "same-origin",
  });
  if (!session.ok) {
    return undefined;
  }
  const body: unknown = await session.json();
  const csrfToken = csrfTokenOf(body);
  return csrfToken === undefined ? undefined : { csrfToken, offlineCache: offlineCacheOf(body) };
}

/** Ends the page's sign-in at `POST /sign-out`, with its CSRF token. */
export async function signOut(
  page: Pick<SignInPage, "location" | "fetch">,
  signedIn: SignedIn,
): Promise<void> {
  await page.fetch(new URL("/sign-out", page.location.origin), {
    method: "POST",
    credentials: "same-origin",
    headers: { "X-CSRF-Token": signedIn.csrfToken },
  });
}
