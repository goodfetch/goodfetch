// Golden scenario 04, "Log in once with a person, reuse the login headless": the leader, Playwright
// 1.64.1 (TypeScript), as a library script with `page.pause()` and `storageState`.
// `node 04-login-human-pause.playwright.js` logs in with a person; `--reuse` checks the kept login headless.
import { chromium } from "playwright";

// The authentication guide's location; its advice is to keep the folder out of version control.
const STATE = "playwright/.auth/quotes.json";

async function logInOnce(username: string): Promise<void> {
  const password = process.env.QUOTES_PASSWORD;
  if (password === undefined) throw new Error("Set QUOTES_PASSWORD");
  // Ctrl+C: Playwright's own SIGINT handler closes the browser (`handleSIGINT`, on by default).
  const browser = await chromium.launch({ headless: false });
  try {
    const context = await browser.newContext();
    const page = await context.newPage();
    await page.goto("https://quotes.toscrape.com/login");
    await page.getByLabel("Username").fill(username);
    // The page's "Password" label points at the username field (`for="username"`): the field is found by its id.
    await page.locator("#password").fill(password);
    // The person checks the form, presses Login, then Resume in the Playwright Inspector; no timeout applies.
    await page.pause();
    await page.getByRole("link", { name: "Logout" }).waitFor();
    await context.storageState({ path: STATE });
  } finally {
    await browser.close();
  }
}

async function confirmLogin(): Promise<"logged_in" | "expired"> {
  const browser = await chromium.launch();
  try {
    const context = await browser.newContext({ storageState: STATE });
    const page = await context.newPage();
    await page.goto("https://quotes.toscrape.com/");
    const logout = page.getByRole("link", { name: "Logout" });
    // A logged-out page shows the Login link instead.
    await logout.or(page.getByRole("link", { name: "Login" })).waitFor();
    return (await logout.isVisible()) ? "logged_in" : "expired";
  } finally {
    await browser.close();
  }
}

if (process.argv.includes("--reuse")) {
  const state = await confirmLogin();
  console.log(
    state === "logged_in"
      ? "Logged in: the page shows Logout."
      : "The kept login has expired: log in again.",
  );
} else {
  await logInOnce("alice");
}
