// Log in to quotes.toscrape.com once with a person at the keyboard, keep the login as a Saved identity,
// then reuse it in a headless run.
//
//   QUOTES_USERNAME=... QUOTES_PASSWORD=... node 04-login-human-pause.ts login
//   node 04-login-human-pause.ts check
//
// The password is never read by this program: the engine resolves `env:QUOTES_PASSWORD` at the moment
// of use and types it into the field. The program only names the secret by a placeholder.

import { Collie, secret } from "@goodfetch/collie";
import type { GoodfetchError, Logger } from "@goodfetch/collie";

const IDENTITY_NAME = "quotes-toscrape";
const LOGIN_URL = "https://quotes.toscrape.com/login";
const HOME_URL = "https://quotes.toscrape.com/";

// The logger only ever receives messages written here, none of which holds a secret value.
const logger: Logger = {
  debug: () => {},
  info: (message) => console.error(`[info] ${message}`),
  warn: (message) => console.error(`[warn] ${message}`),
  error: (message) => console.error(`[error] ${message}`),
};

function isError(error: unknown, ...codes: string[]): error is GoodfetchError {
  return (
    error instanceof Error &&
    "code" in error &&
    typeof error.code === "string" &&
    codes.includes(error.code)
  );
}

/** Run 1: a visible browser, a person checks the form and presses Login. */
async function login(): Promise<number> {
  const username = process.env["QUOTES_USERNAME"];
  if (username === undefined || username === "") {
    console.error("Set QUOTES_USERNAME to the username to fill in.");
    return 2;
  }

  // Cancel with Ctrl+C: the abort ends the Human pause (and the whole wait) cleanly.
  const cancel = new AbortController();
  const onSigint = () => cancel.abort(new Error("cancelled by the person"));
  process.once("SIGINT", onSigint);

  await using collie = await Collie.connect({ logger });
  try {
    await using run = await collie.start({
      id: "quotes-login",
      timeout: null,
      signal: cancel.signal,
      settings: {
        browser: { headless: false },
        // No limit while the person acts.
        timeout: { humanPause: null },
        // `secrets.quotes_password`: the engine reads the environment variable itself, and only
        // types it into a field whose frame is on toscrape.com.
        secrets: {
          quotes_password: { reference: "env:QUOTES_PASSWORD", domains: ["toscrape.com"] },
        },
      },
    });

    const page = await run.newPage();
    await page.goto(LOGIN_URL);
    await page.getByLabel("Username").fill(username);
    await page.getByLabel("Password").fill(secret("quotes_password"));

    const logout = page.getByRole("link", { name: "Logout" });
    console.error("Check the form in the browser window, then press Login. Ctrl+C cancels.");
    const result = await page.pause({
      reason: "Check the login form and press Login",
      // Ends by itself once the logged-in page shows a Logout link; a person's "Done" counts only then.
      until: logout,
      timeout: null,
      signal: cancel.signal,
    });
    console.error(`Login detected (${result.resolvedBy}, waited ${Math.round(result.waited / 1000)} s).`);

    const saved = await page.session().saveIdentity(IDENTITY_NAME);
    console.error(`Saved the login as identity "${saved.name}".`);
    await run.finish();
    return 0;
  } catch (error) {
    if (cancel.signal.aborted) {
      console.error("Cancelled: no login was kept.");
      return 130;
    }
    if (isError(error, "human_pause_declined")) {
      console.error("The person declined the login step: no login was kept.");
      return 1;
    }
    if (isError(error, "secret_unavailable")) {
      console.error("The secret env:QUOTES_PASSWORD is not available: set QUOTES_PASSWORD in the engine's environment.");
      return 2;
    }
    throw error;
  } finally {
    process.off("SIGINT", onSigint);
  }
}

/** Run 2: headless, reuse the kept login and confirm the Logout link. */
async function check(): Promise<number> {
  await using collie = await Collie.connect({ logger });

  const kept = await collie.identities.list();
  const identity = kept.find((candidate) => candidate.name === IDENTITY_NAME);
  if (identity === undefined) {
    console.error(`No kept login named "${IDENTITY_NAME}": run the login step first.`);
    return 1;
  }

  await using run = await collie.start({
    settings: { browser: { headless: true } },
  });
  try {
    await using session = await run.newSession({
      source: { kind: "saved_identity", name: IDENTITY_NAME },
      settings: {
        // Nobody is at the keyboard in this run: a lost login must not wait for a person or an agent.
        session: { recovery: { humanPause: false, agent: false } },
      },
    });
    const page = await session.newPage();
    await page.goto(HOME_URL);

    const logout = page.getByRole("link", { name: "Logout" });
    if ((await logout.count()) > 0) {
      console.error("Logged in: the page shows a Logout link.");
      await run.finish();
      return 0;
    }

    const loginLink = page.getByRole("link", { name: "Login" });
    const sawLogin = (await loginLink.count()) > 0;
    console.error(
      `The kept login has expired${sawLogin ? " (the page offers a Login link)" : ""}: run the login step again.`,
    );
    return 3;
  } catch (error) {
    if (isError(error, "logged_out", "page_check_failed")) {
      console.error("The kept login has expired: run the login step again.");
      return 3;
    }
    throw error;
  }
}

const mode = process.argv[2];
if (mode === "login") {
  process.exitCode = await login();
} else if (mode === "check") {
  process.exitCode = await check();
} else {
  console.error("Usage: 04-login-human-pause.ts <login|check>");
  process.exitCode = 2;
}
