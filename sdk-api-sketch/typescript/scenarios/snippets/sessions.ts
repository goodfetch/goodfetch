// Snippet for Sessions, logins, the Human pause and secrets in code (#476), on golden scenario 04.
import { Goodfetch, HumanPauseDeclinedError, LoggedOutError, secret, type Logger } from "goodfetch";

declare const log: Logger; // the program's own logger
declare const vault: { read(name: string): Promise<string> }; // the program's own secret store

// First run: a person at the keyboard logs in once, and the login is kept under a name.
export async function logInOnce(): Promise<void> {
  await using gf = await Goodfetch.connect();
  await using run = await gf.collie.start({
    settings: {
      // A Secret placeholder: the program names it, and the engine types its value, only into a field on toscrape.com.
      secrets: { password: { reference: "env:QUOTES_PASSWORD", domains: ["toscrape.com"] } },
    },
  });
  // A visible window for the person; headless, they would act in the Live view instead.
  await using session = await run.newSession({ settings: { browser: { headless: false } } });
  const page = await session.newPage();
  await page.goto("https://quotes.toscrape.com/login");
  await page.getByLabel("Username").fill("alice");
  await page.getByLabel("Password").fill(secret("password"));
  try {
    // The Human pause: no clock runs while it waits, and it ends once the Logout link shows.
    await page.pause({
      reason: "Check the form, then press Login",
      until: page.getByRole("link", { name: "Logout" }),
      timeout: null,
    });
  } catch (error) {
    if (error instanceof HumanPauseDeclinedError) log.warn("Login declined", { note: error.details.note });
    throw error;
  }
  // Kept encrypted per install, never as a clear-text file (ADRs 0035, 0056).
  await session.saveIdentity("quotes");
}

// Second run: headless and unattended, it reuses the login and says when it has expired.
export async function reuseLogin(): Promise<boolean> {
  await using gf = await Goodfetch.connect();
  await using run = await gf.collie.start();
  await using session = await run.newSession({
    source: { kind: "saved_identity", name: "quotes" },
    settings: {
      session: {
        // A page showing the Login link is a lost login; nobody is there to log in again.
        pageCheck: { loginPage: { selector: "a[href='/login']" } },
        recovery: { humanPause: false },
      },
    },
  });
  const page = await session.newPage();
  try {
    await page.goto("https://quotes.toscrape.com/");
  } catch (error) {
    if (error instanceof LoggedOutError) {
      log.warn("The saved login has expired: run logInOnce again", { steps: error.details.steps.length });
      return false;
    }
    throw error;
  }
  return page.getByRole("link", { name: "Logout" }).isVisible();
}

// Cookies, a proxy and secrets the program answers from its own store.
export async function cookiesAndProxy(): Promise<number> {
  await using gf = await Goodfetch.connect({ resolveSecret: async ({ name }) => vault.read(name) });
  await using run = await gf.collie.start({
    settings: {
      proxy: { server: "http://proxy.example:3128", username: "crawler", password: "caller:proxy_password" },
      session: { httpCredentials: [{ username: "reader", password: "caller:basic_password", domains: ["httpbin.org"] }] },
      secrets: { token: { reference: "caller:api_token", domains: ["toscrape.com"] } },
    },
  });
  // The Run's default Session, which every `run.newPage()` shares.
  const page = await run.newPage();
  const session = page.session();
  await session.addCookies([{ name: "theme", value: "dark", url: "https://quotes.toscrape.com" }]);
  await page.goto("https://quotes.toscrape.com/");
  await page.request.get("https://quotes.toscrape.com/api/quotes", { headers: { authorization: secret("token") } });
  const cookies = await session.cookies("https://quotes.toscrape.com");

  // @ts-expect-error No field to check: keyboard input takes no Secret placeholder.
  await page.keyboard.type(secret("token"));
  // @ts-expect-error A credential the engine uses itself is a Secret reference, never a value.
  await gf.collie.start({ settings: { proxy: { password: "hunter2" } } });
  // @ts-expect-error Opening a Session spells its source as `source`, one spelling (S2).
  await run.newSession({ settings: { session: { source: { kind: "saved_identity", name: "quotes" } } } });

  for (const identity of await gf.identities.list()) log.info("Saved identity", { name: identity.name, cookiesExpire: identity.cookiesExpire });
  return cookies.length;
}
