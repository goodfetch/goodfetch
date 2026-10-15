/// <reference types="node" />
// Golden scenario 04, "Log in once with a person, reuse the login headless" (Collie, TypeScript).
// `node 04-login-human-pause.js` logs in with a person; `--reuse` confirms the kept login headless.
import { Collie, LoggedOutError, secret } from "@goodfetch/collie";

const IDENTITY = "quotes"; // the name the login is kept under

/** First run: a person checks the form and presses Login; the login is kept as a Saved identity. */
export async function logInOnce(username: string, signal: AbortSignal): Promise<void> {
  await using collie = await Collie.connect();
  await using run = await collie.start({
    settings: {
      // The Secret placeholder `password`: the engine reads QUOTES_PASSWORD and types it only into a
      // field on toscrape.com. The program never holds it, and every log, trace, screenshot and
      // returned value shows `%password%` instead (ADR 0092).
      secrets: { password: { reference: "env:QUOTES_PASSWORD", domains: ["toscrape.com"] } },
    },
  });
  const session = await run.newSession({ settings: { browser: { headless: false } } });
  const page = await session.newPage();
  await page.goto("https://quotes.toscrape.com/login");
  await page.getByLabel("Username").fill(username);
  // The page's "Password" label points at the username field (`for="username"`): the field is found by its id.
  await page.locator("#password").fill(secret("password"));
  // The Human pause: no time limit and no other clock runs; it ends once the Logout link shows, a
  // person's "Decline" throws `HumanPauseDeclinedError`, and the signal cancels it. Closing the window
  // throws `CallCancelledError` with `details.closed === "page"` (ADR 0113).
  await page.pause({
    reason: "Check the form, then press Login",
    until: page.getByRole("link", { name: "Logout" }),
    timeout: null,
    signal,
  });
  // Kept encrypted in the install's store under that name, never as a clear-text file (ADR 0056).
  await session.saveIdentity(IDENTITY);
  await run.finish();
}

/** Second run, headless by default: `expired` when the kept login no longer logs in. */
export async function confirmLogin(): Promise<"logged_in" | "expired"> {
  await using collie = await Collie.connect();
  await using run = await collie.start();
  const session = await run.newSession({
    source: { kind: "saved_identity", name: IDENTITY },
    settings: {
      session: {
        // A page showing the Login link is a lost login, and no one is there to log in again.
        pageCheck: { loginPage: { selector: "a[href='/login']" } },
        recovery: { humanPause: false, agent: false },
      },
    },
  });
  const page = await session.newPage();
  let state: "logged_in" | "expired" = "logged_in";
  try {
    await page.goto("https://quotes.toscrape.com/");
    await page.getByRole("link", { name: "Logout" }).waitFor();
  } catch (error) {
    if (!(error instanceof LoggedOutError)) throw error;
    state = "expired"; // an outcome, named by the Page check, not a missing element
  }
  await run.finish();
  return state;
}

if (process.argv.includes("--reuse")) {
  const state = await confirmLogin();
  console.log(
    state === "logged_in"
      ? "Logged in: the page shows Logout."
      : "The kept login has expired: log in again.",
  );
} else {
  const cancel = new AbortController();
  process.once("SIGINT", () => cancel.abort()); // Ctrl+C cancels the wait for the person
  await logInOnce("alice", cancel.signal);
}
