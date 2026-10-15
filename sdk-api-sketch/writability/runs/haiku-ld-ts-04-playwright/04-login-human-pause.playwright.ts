// Login with a person at the keyboard, then reuse the kept login state headlessly.
//
//   QUOTES_TOSCRAPE_PASSWORD=... node 04-login-human-pause.playwright.ts login
//   node 04-login-human-pause.playwright.ts verify
//
// The password is read from an environment variable only when filling the form.
// It is never printed, never written to a file, and no tracing is enabled.

import { access, chmod, mkdir, readFile, rename, rm } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { chromium, type BrowserContext } from 'playwright';

const LOGIN_URL = 'https://quotes.toscrape.com/login';
const HOME_URL = 'https://quotes.toscrape.com/';
const USERNAME = 'admin';
const PASSWORD_ENV = 'QUOTES_TOSCRAPE_PASSWORD';
const STATE_NAME = 'quotes-toscrape';
const STATE_DIR = join(homedir(), '.local', 'state', 'goodfetch-quotes-login');
const STATE_PATH = join(STATE_DIR, `${STATE_NAME}.storage-state.json`);
const TMP_STATE_PATH = `${STATE_PATH}.tmp`;

type LoginOutcome = 'logged-in' | 'cancelled' | 'closed';

function readPassword(): string {
  const password = process.env[PASSWORD_ENV];
  if (!password) {
    throw new Error(`Set the ${PASSWORD_ENV} environment variable to the password, then run the login mode again.`);
  }
  return password;
}

function reportError(error: unknown, secret?: string): void {
  let message = error instanceof Error ? error.message : String(error);
  if (secret) {
    message = message.split(secret).join('[redacted]');
  }
  console.error(message);
}

// Resolves when the person types "cancel" and presses Enter in this terminal, or presses Ctrl+C.
function waitForCancel(): { cancelled: Promise<LoginOutcome>; dispose: () => void } {
  let dispose: () => void = () => {};
  const cancelled = new Promise<LoginOutcome>((resolve) => {
    const rl = createInterface({ input: process.stdin, output: process.stdout });
    const onCancel = (): void => resolve('cancelled');
    rl.on('line', (line) => {
      if (line.trim().toLowerCase() === 'cancel') {
        onCancel();
      }
    });
    rl.on('SIGINT', onCancel);
    process.once('SIGINT', onCancel);
    dispose = () => {
      process.off('SIGINT', onCancel);
      rl.close();
    };
  });
  return { cancelled, dispose };
}

async function keepState(context: BrowserContext, password: string): Promise<void> {
  await mkdir(STATE_DIR, { recursive: true, mode: 0o700 });
  await context.storageState({ path: TMP_STATE_PATH });
  await chmod(TMP_STATE_PATH, 0o600);
  const saved = await readFile(TMP_STATE_PATH, 'utf8');
  if (saved.includes(password)) {
    await rm(TMP_STATE_PATH, { force: true });
    throw new Error('The login state contains the password, so it was not kept.');
  }
  await rename(TMP_STATE_PATH, STATE_PATH);
}

async function loginWithPause(password: string): Promise<number> {
  const browser = await chromium.launch({ headless: false });
  const cancel = waitForCancel();
  try {
    const context = await browser.newContext();
    const page = await context.newPage();
    await page.goto(LOGIN_URL, { waitUntil: 'load' });
    await page.locator('#username').fill(USERNAME);
    await page.locator('#password').fill(password);
    console.log('The login form is filled in the browser window. Check it and press Login there.');
    console.log('To cancel, type "cancel" and press Enter here, or press Ctrl+C.');

    // No time limit: wait until the logged-in page shows a Logout link, or the window is closed.
    const loggedIn: Promise<LoginOutcome> = page
      .getByRole('link', { name: 'Logout', exact: true })
      .waitFor({ state: 'attached', timeout: 0 })
      .then(
        (): LoginOutcome => 'logged-in',
        (error: unknown): LoginOutcome => {
          if (page.isClosed()) {
            return 'closed';
          }
          throw error;
        },
      );
    const outcome = await Promise.race([loggedIn, cancel.cancelled]);

    if (outcome !== 'logged-in') {
      console.log(
        outcome === 'cancelled'
          ? 'Cancelled. No login state was kept.'
          : 'The browser closed before the login finished. No login state was kept.',
      );
      return 1;
    }
    await keepState(context, password);
    console.log(`Logged-in state kept as "${STATE_NAME}".`);
    return 0;
  } finally {
    cancel.dispose();
    await browser.close().catch(() => undefined);
    await rm(TMP_STATE_PATH, { force: true });
  }
}

async function login(): Promise<number> {
  const password = readPassword();
  try {
    return await loginWithPause(password);
  } catch (error) {
    reportError(error, password);
    return 1;
  }
}

async function verify(): Promise<number> {
  const hasState = await access(STATE_PATH).then(
    () => true,
    () => false,
  );
  if (!hasState) {
    console.error(`No kept login state "${STATE_NAME}" was found. Run the login mode first.`);
    return 2;
  }

  const browser = await chromium.launch({ headless: true });
  try {
    const context = await browser.newContext({ storageState: STATE_PATH });
    const page = await context.newPage();
    await page.goto(HOME_URL, { waitUntil: 'load' });
    const logoutLinks = await page.getByRole('link', { name: 'Logout', exact: true }).count();
    if (logoutLinks === 0) {
      console.log(
        `The kept login state "${STATE_NAME}" has expired: the page shows no Logout link. Run the login mode again.`,
      );
      return 2;
    }
    console.log('Logged in with the kept state: the page shows a Logout link.');
    return 0;
  } finally {
    await browser.close().catch(() => undefined);
  }
}

async function main(): Promise<number> {
  const mode = process.argv[2];
  if (mode === 'login') {
    return login();
  }
  if (mode === 'verify') {
    return verify();
  }
  console.error('Usage: node 04-login-human-pause.playwright.ts <login|verify>');
  return 64;
}

main().then(
  (code) => {
    process.exitCode = code;
  },
  (error: unknown) => {
    reportError(error);
    process.exitCode = 1;
  },
);
