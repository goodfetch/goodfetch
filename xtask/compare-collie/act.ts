// One trial of the Collie scenario of `just compare` (issue 2282): Collie's
// TypeScript SDK, over its Attached engine's JSON-RPC, or Playwright, each
// attached to the Chrome that `xtask compare` launched, through the counting
// relay at `--cdp`, carries out the plan `--plan` names as `plan.ts` does
// and prints one JSON line of what each page came to. A Session is Collie's
// `run.newSession` with a `cdp_url` source, or Playwright's
// `browser.newContext()`; starting Collie's engine and Playwright's
// `connectOverCDP` stay outside the marks.
//
//   node act.ts --tool collie --plan plan.json \
//     --cdp ws://127.0.0.1:<relay>/devtools/browser/<id> \
//     --marks http://127.0.0.1:<relay>
import {
  Collie,
  type Page as CollieDriverPage,
  type Session as CollieSession,
} from "@goodfetch/collie";
import {
  chromium,
  type Browser,
  type BrowserContext,
  type Page as PlaywrightDriverPage,
} from "playwright";

import {
  type Driver,
  type DriverPage,
  type DriverSession,
  type Plan,
  main,
  READ,
  readText,
} from "./plan.ts";

/** A Collie page as any tool's. */
function colliePage(opened: CollieDriverPage): DriverPage {
  return {
    async goto(url) {
      const navigated = await opened.goto(url);
      return navigated.response?.status ?? null;
    },
    async step(step) {
      if (step.action === "read") {
        const evaluated = await opened.evaluate(READ);
        return readText(evaluated.value);
      }
      const target = opened.locator(step.selector);
      if (step.action === "click") await target.click();
      else if (step.action === "fill") await target.fill(step.value);
      else await target.press(step.key);
      return null;
    },
    close: () => opened.close(),
  };
}

/** A Collie Session as any tool's. */
function collieSession(opened: CollieSession): DriverSession {
  return {
    newPage: async () => colliePage(await opened.newPage()),
    close: () => opened.close(),
  };
}

async function collie(cdp: string): Promise<Driver> {
  // The process's environment, which xtask set: the trial's Goodfetch
  // home, whose user configuration maps the Proving ground's hosts, and the
  // engine's path.
  const connection = await Collie.connect();
  const run = await connection.start();
  return {
    newSession: async () =>
      collieSession(await run.newSession({ source: { kind: "cdp_url", url: cdp } })),
    async close() {
      await run.finish();
      await connection.close();
    },
  };
}

async function playwright(plan: Plan, cdp: string): Promise<Driver> {
  const browser: Browser = await chromium.connectOverCDP(cdp);
  const page = (opened: PlaywrightDriverPage): DriverPage => ({
    async goto(url) {
      const response = await opened.goto(url);
      return response?.status() ?? null;
    },
    async step(step) {
      if (step.action === "read") return readText(await opened.evaluate<unknown>(READ));
      const timeout = plan.action_timeout;
      const target = opened.locator(step.selector);
      if (step.action === "click") await target.click({ timeout });
      else if (step.action === "fill") await target.fill(step.value, { timeout });
      else await target.press(step.key, { timeout });
      return null;
    },
    close: () => opened.close(),
  });
  const session = (opened: BrowserContext): DriverSession => ({
    newPage: async () => page(await opened.newPage()),
    close: () => opened.close(),
  });
  return {
    newSession: async () => session(await browser.newContext()),
    close: () => browser.close(),
  };
}

await main({
  collie: (_plan, cdp) => collie(cdp),
  playwright,
});
