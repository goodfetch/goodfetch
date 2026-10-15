// Keeps a quotes.toscrape.com login that a person makes in a visible browser, then reuses it headless.
//
//   node 04-login-human-pause.ts login   visible browser; the person checks the form and presses Login
//   node 04-login-human-pause.ts check   headless; reuses the kept login and confirms a Logout link
//
// The program never holds the password. It names the secret placeholder `password`, which the engine
// binds to the environment variable QUOTES_PASSWORD and types into the field. The username comes from
// QUOTES_USERNAME. Neither value is logged or saved. The kept login is a Saved identity, which the SDK
// keeps encrypted in the install, never as a clear-text file. This program never writes a trace or a file.

import { Collie, LoggedOutError, secret } from "@goodfetch/collie";
import type { Page, Run, RunSettings, SecretBinding, SessionSettings, SessionSource } from "@goodfetch/collie";

const LOGIN_URL = "https://quotes.toscrape.com/login";
const HOME_URL = "https://quotes.toscrape.com/";
const SITE_DOMAIN = "toscrape.com";
const SAVED_NAME = "quotes-toscrape";
const PASSWORD_PLACEHOLDER = "password";

const EXIT_OK = 0;
const EXIT_FAILED = 1;
const EXIT_NO_KEPT_LOGIN = 2;
const EXIT_EXPIRED = 3;
const EXIT_CANCELLED = 130;

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

// The engine resolves `env:` at the moment of use and types the value; the program holds only the reference.
const passwordBinding: SecretBinding = { reference: "env:QUOTES_PASSWORD", domains: [SITE_DOMAIN] };

const loginRunSettings: RunSettings = {
  browser: { headless: false },
  secrets: { [PASSWORD_PLACEHOLDER]: passwordBinding },
};

const keptLogin: SessionSource = { kind: "saved_identity", name: SAVED_NAME };

// No human pause and no agent login during recovery: a headless run must end with a clear answer, not wait for a person.
const checkSessionSettings: SessionSettings = {
  session: { recovery: { humanPause: false, agent: false } },
};

async function login(): Promise<number> {
  const username = process.env.QUOTES_USERNAME ?? "demo";
  const cancel = new AbortController();
  const onInterrupt = (): void => cancel.abort(new Error("Login cancelled at the keyboard."));
  process.once("SIGINT", onInterrupt);

  try {
    await using collie = await Collie.connect();
    await using run = await collie.start({ settings: loginRunSettings, signal: cancel.signal });
    const page: Page = await run.newPage();
    await page.goto(LOGIN_URL);
    await page.getByLabel("Username", { exact: true }).fill(username);
    await page.getByLabel("Password", { exact: true }).fill(secret(PASSWORD_PLACEHOLDER));

    console.log("Check the form in the browser window and press Login. No time limit applies; press Ctrl+C to cancel.");
    const logout = page.getByRole("link", { name: "Logout", exact: true });
    const paused = await page.pause({
      reason: "Check the username and password, then press Login.",
      until: logout,
      timeout: null,
      signal: cancel.signal,
    });
    console.log(`Logged in (${paused.resolvedBy}). Keeping the login as "${SAVED_NAME}".`);

    await page.session().saveIdentity(SAVED_NAME, { signal: cancel.signal });
    await run.finish();
    console.log(`Kept the login as "${SAVED_NAME}".`);
    return EXIT_OK;
  } catch (error) {
    if (cancel.signal.aborted) {
      console.log("Login cancelled. No login was kept.");
      return EXIT_CANCELLED;
    }
    console.log(`Login failed: ${messageOf(error)}`);
    return EXIT_FAILED;
  } finally {
    process.off("SIGINT", onInterrupt);
  }
}

// True when the page shows a Logout link; false when the kept login has lost its session.
async function keptLoginIsValid(run: Run): Promise<boolean> {
  try {
    const session = await run.newSession({ source: keptLogin, settings: checkSessionSettings });
    const page = await session.newPage();
    await page.goto(HOME_URL);
    // isVisible answers at once, so an expired login is reported rather than waited for.
    return await page.getByRole("link", { name: "Logout", exact: true }).isVisible();
  } catch (error) {
    if (error instanceof LoggedOutError) {
      return false;
    }
    throw error;
  }
}

async function check(): Promise<number> {
  await using collie = await Collie.connect();
  const kept = (await collie.identities.list()).some((identity) => identity.name === SAVED_NAME);
  if (!kept) {
    console.log(`No kept login named "${SAVED_NAME}". Run the login mode first.`);
    return EXIT_NO_KEPT_LOGIN;
  }

  await using run = await collie.start({ settings: { browser: { headless: true } } });
  const valid = await keptLoginIsValid(run);
  await run.finish();

  if (valid) {
    console.log(`Logged in with "${SAVED_NAME}": the page shows a Logout link.`);
    return EXIT_OK;
  }
  console.log(`The kept login "${SAVED_NAME}" has expired: ${HOME_URL} shows no Logout link. Run the login mode again.`);
  return EXIT_EXPIRED;
}

async function main(): Promise<number> {
  const mode = process.argv[2];
  try {
    if (mode === "login") {
      return await login();
    }
    if (mode === "check") {
      return await check();
    }
  } catch (error) {
    console.log(`Failed: ${messageOf(error)}`);
    return EXIT_FAILED;
  }
  console.log("Usage: node 04-login-human-pause.ts login | check");
  return EXIT_FAILED;
}

process.exitCode = await main();
