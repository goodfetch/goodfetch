// One trial of the Collie scenario of `just compare` with one of the arms
// (issues 2443 and 2441): Puppeteer, through its locators or its page
// methods, or Stagehand with no model, each attached to the Chrome that
// `xtask compare` launched, through the counting relay at `--cdp`. It
// carries out the plan `--plan` names as `../compare-collie/plan.ts` does
// and prints one JSON line of what each page came to.
//
// A Session is Puppeteer's `browser.createBrowserContext()`. Stagehand
// drives the browser's one context, so it opens no Session of its own and
// xtask plans it no Sessions cell. Connecting stays outside the marks.
//
//   node act.ts --tool puppeteer_locator --plan plan.json \
//     --cdp ws://127.0.0.1:<relay>/devtools/browser/<id> \
//     --marks http://127.0.0.1:<relay>
import {
  localBrowser,
  Stagehand,
  type Page as StagehandDriverPage,
} from "@browserbasehq/stagehand";
import {
  connect,
  type Browser,
  type BrowserContext,
  type KeyInput,
  type Page as PuppeteerDriverPage,
} from "puppeteer-core";

import {
  type Driver,
  type DriverPage,
  type DriverSession,
  type Plan,
  type Step,
  main,
  READ,
  readText,
} from "../compare-collie/plan.ts";

/** The keys the plan presses (xtask's `Step::Press`), as Puppeteer names them. */
const KEYS = ["Enter", "e"] as const satisfies readonly KeyInput[];

/**
 * A plan's key as Puppeteer's key name. Puppeteer types its key names, and
 * exports no list of them to check one against, so a key the plan gains
 * fails its step here, naming it.
 */
function keyInput(key: string): KeyInput {
  const known = KEYS.find((name) => name === key);
  if (known === undefined) throw new Error(`the plan presses ${key}, which act.ts does not name`);
  return known;
}

/** How a Puppeteer row takes a click, fill or press step. */
type PuppeteerAct = (
  page: PuppeteerDriverPage,
  step: Exclude<Step, { readonly action: "read" }>,
  timeout: number,
) => Promise<void>;

/**
 * Puppeteer's locators: each action waits for its target to be in the
 * viewport, enabled and stable, up to the plan's timeout, though not for it
 * to be visible or uncovered. A locator has no press, so a press takes the
 * handle the locator waited for.
 */
const throughLocators: PuppeteerAct = async (page, step, timeout) => {
  const target = page.locator(step.selector).setTimeout(timeout);
  if (step.action === "click") await target.click();
  else if (step.action === "fill") await target.fill(step.value);
  else {
    const handle = await target.waitHandle();
    try {
      await handle.press(keyInput(step.key));
    } finally {
      await handle.dispose();
    }
  }
};

/**
 * Puppeteer's page methods, which act on the first element the selector
 * matches when they are called and fail at once when none does.
 */
const throughPageMethods: PuppeteerAct = async (page, step) => {
  if (step.action === "click") await page.click(step.selector);
  else if (step.action === "fill") await page.type(step.selector, step.value);
  else {
    await page.focus(step.selector);
    await page.keyboard.press(keyInput(step.key));
  }
};

async function puppeteerDriver(plan: Plan, cdp: string, act: PuppeteerAct): Promise<Driver> {
  const browser: Browser = await connect({ browserWSEndpoint: cdp });
  const page = (opened: PuppeteerDriverPage): DriverPage => ({
    async goto(url) {
      const response = await opened.goto(url);
      return response?.status() ?? null;
    },
    async step(step) {
      if (step.action === "read") return readText(await opened.evaluate(READ));
      await act(opened, step, plan.action_timeout);
      return null;
    },
    close: () => opened.close(),
  });
  const session = (opened: BrowserContext): DriverSession => ({
    newPage: async () => page(await opened.newPage()),
    close: () => opened.close(),
  });
  return {
    newSession: async () => session(await browser.createBrowserContext()),
    close: () => browser.disconnect(),
  };
}

async function stagehand(plan: Plan, cdp: string): Promise<Driver> {
  // Stagehand with no model: its page and locator calls only, which ask
  // for none. Stagehand loads its extension into the browser, which needs
  // the flags xtask gives Stagehand's trials.
  const browser = await localBrowser.connect({ cdpUrl: cdp });
  await Stagehand.create({ browser, logging: { level: "error" } });
  const page = (opened: StagehandDriverPage): DriverPage => ({
    async goto(url) {
      const response = await opened.goto(url);
      return response?.status() ?? null;
    },
    async step(step) {
      if (step.action === "read") return readText(await opened.evaluate(READ));
      const timeout = plan.action_timeout;
      if (step.action === "click") await opened.locator(step.selector).click({ timeout });
      else if (step.action === "fill") {
        await opened.locator(step.selector).fill(step.value, { timeout });
      } else {
        // A Stagehand locator has no press: the page presses a key where
        // the focus is.
        await opened.evaluate(`document.querySelector(${JSON.stringify(step.selector)}).focus()`);
        await opened.keyPress(step.key);
      }
      return null;
    },
    close: () => opened.close(),
  });
  // The browser's one context: closing it would close the browser's
  // default context, so the Session's close leaves it.
  const session: DriverSession = {
    newPage: async () => page(await browser.context.newPage()),
    close: async () => {},
  };
  return {
    newSession: async () => session,
    close: () => browser.disconnect(),
  };
}

await main({
  puppeteer_locator: (plan, cdp) => puppeteerDriver(plan, cdp, throughLocators),
  puppeteer_page: (plan, cdp) => puppeteerDriver(plan, cdp, throughPageMethods),
  stagehand,
});
