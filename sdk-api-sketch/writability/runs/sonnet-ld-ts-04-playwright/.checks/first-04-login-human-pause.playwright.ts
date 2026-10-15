// Log in to quotes.toscrape.com once with a person at the keyboard, keep the
// logged-in state under a name, then reuse it in a second, headless run.
//
//   QUOTES_PASSWORD=... node 04-login-human-pause.playwright.ts login [state-name]
//   node 04-login-human-pause.playwright.ts check [state-name]
//
// Exit codes: 0 success, 1 error, 2 no kept state, 3 kept state expired, 130 cancelled.

import { mkdir, chmod, access } from "node:fs/promises";
import { createInterface } from "node:readline";
import path from "node:path";
import { chromium, type Browser, type Page } from "playwright";

const BASE_URL = "https://quotes.toscrape.com/";
const LOGIN_URL = `${BASE_URL}login`;
const USERNAME = process.env["QUOTES_USERNAME"] ?? "demo";
const PASSWORD_ENV = "QUOTES_PASSWORD";
const STATE_DIR = path.resolve(".auth");
const DEFAULT_STATE_NAME = "quotes-login";

class Cancelled extends Error {}

function log(message: string): void {
  console.log(`[quotes] ${message}`);
}

// The secret is read at the moment it is typed into the field and is never
// stored in a variable, logged, traced or written to disk by this program.
function readSecret(): string {
  const value = process.env[PASSWORD_ENV];
  if (value === undefined || value === "") {
    throw new Error(`Set the ${PASSWORD_ENV} environment variable before running the login step.`);
  }
  return value;
}

function stateFile(name: string): string {
  if (!/^[A-Za-z0-9_-]+$/.test(name)) {
    throw new Error("The state name may only contain letters, digits, '-' and '_'.");
  }
  return path.join(STATE_DIR, `${name}.json`);
}

// Playwright's API debug log prints the arguments of fill(), including the password.
function refuseApiDebugLogging(): void {
  const debug = process.env["DEBUG"] ?? "";
  if (/pw:|playwright/i.test(debug) || debug === "*") {
    throw new Error("Unset DEBUG: Playwright's debug log would print the typed password.");
  }
}

async function exists(file: string): Promise<boolean> {
  try {
    await access(file);
    return true;
  } catch {
    return false;
  }
}

// Resolves to a promise that rejects with Cancelled when the person cancels:
// Ctrl+C, SIGTERM, typing "cancel" on the terminal, or closing the browser window.
function cancellation(browser: Browser): { promise: Promise<never>; dispose: () => void } {
  let reject: (reason: Cancelled) => void = () => undefined;
  const promise = new Promise<never>((_resolve, rej) => {
    reject = rej;
  });
  promise.catch(() => undefined);

  const cancel = (why: string) => () => reject(new Cancelled(why));
  const onSigint = cancel("interrupted");
  const onSigterm = cancel("terminated");
  const onClosed = cancel("the browser window was closed");
  process.once("SIGINT", onSigint);
  process.once("SIGTERM", onSigterm);
  browser.once("disconnected", onClosed);

  const rl = createInterface({ input: process.stdin });
  rl.on("line", (line) => {
    if (line.trim().toLowerCase() === "cancel") reject(new Cancelled("cancelled from the terminal"));
  });

  return {
    promise,
    dispose: () => {
      process.off("SIGINT", onSigint);
      process.off("SIGTERM", onSigterm);
      browser.off("disconnected", onClosed);
      rl.close();
    },
  };
}

async function showsLogout(page: Page): Promise<boolean> {
  return (await page.getByRole("link", { name: "Logout" }).count()) > 0;
}

async function login(name: string): Promise<number> {
  refuseApiDebugLogging();
  const file = stateFile(name);
  readSecret(); // fail early, before opening a browser, if the secret is missing

  const browser = await chromium.launch({ headless: false });
  const cancel = cancellation(browser);
  try {
    const context = await browser.newContext(); // no tracing, video or screenshots
    const page = await context.newPage();
    await page.goto(LOGIN_URL);

    await page.locator("#username").fill(USERNAME);
    await page.locator("#password").fill(readSecret());

    log("The form is filled. Check it in the browser window and press Login.");
    log("There is no time limit. To cancel: Ctrl+C, type 'cancel' + Enter, or close the window.");

    // Wait, without a timeout, until the site leaves the login page after a successful submit.
    const loggedIn = page.waitForURL((url) => !url.pathname.startsWith("/login"), { timeout: 0 });
    loggedIn.catch(() => undefined);
    await Promise.race([loggedIn, cancel.promise]);

    if (!(await showsLogout(page))) {
      throw new Error("The page left the login form but shows no Logout link; nothing was kept.");
    }

    await mkdir(STATE_DIR, { recursive: true, mode: 0o700 });
    await context.storageState({ path: file }); // cookies and local storage only
    await chmod(file, 0o600);
    log(`Logged in. State kept as "${name}" in ${file}`);
    return 0;
  } finally {
    cancel.dispose();
    await browser.close().catch(() => undefined);
  }
}

async function check(name: string): Promise<number> {
  const file = stateFile(name);
  if (!(await exists(file))) {
    log(`No kept state named "${name}" (looked for ${file}). Run the login step first.`);
    return 2;
  }

  const browser = await chromium.launch({ headless: true });
  try {
    const context = await browser.newContext({ storageState: file });
    const page = await context.newPage();
    await page.goto(BASE_URL);

    if (await showsLogout(page)) {
      log(`Confirmed: the page shows a Logout link. State "${name}" is still valid.`);
      return 0;
    }
    if ((await page.getByRole("link", { name: "Login" }).count()) > 0) {
      log(`The kept state "${name}" has expired: the page shows a Login link, not Logout. Run the login step again.`);
      return 3;
    }
    log("The page shows neither a Logout nor a Login link; cannot tell whether the state is valid.");
    return 1;
  } finally {
    await browser.close().catch(() => undefined);
  }
}

async function main(): Promise<number> {
  const [mode, name = DEFAULT_STATE_NAME] = process.argv.slice(2);
  try {
    if (mode === "login") return await login(name);
    if (mode === "check") return await check(name);
    log("Usage: login [state-name] | check [state-name]");
    return 1;
  } catch (error) {
    if (error instanceof Cancelled) {
      log(`Cancelled (${error.message}). Nothing was kept.`);
      return 130;
    }
    log(`Failed: ${error instanceof Error ? error.message.split("\n")[0] : "unknown error"}`);
    return 1;
  }
}

process.exitCode = await main();
