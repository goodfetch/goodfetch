// Signing in on a person's own machine (ADRs 0024 and 0031): the sign-in
// link carries the Daemon's token in its URL fragment, which no request
// ever sends; the page swaps it once for the Console's `HttpOnly` cookie at
// `POST /sign-in`, then takes it out of the address, so neither the
// address, the history nor a `Referer` keeps it. `GET /session` then says
// whether the page is signed in, with the sign-in's CSRF token.

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

/** A page signed in: the sign-in's CSRF token, which every `POST` of the page carries. */
export interface SignedIn {
  readonly csrfToken: string;
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
  const csrfToken = csrfTokenOf(await session.json());
  return csrfToken === undefined ? undefined : { csrfToken };
}
