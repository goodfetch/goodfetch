// One trial of the Collie scenario of `just compare` (issue 2282): Collie's
// Attached engine over raw JSON-RPC (`raw.ts`), the same engine through
// Collie's TypeScript SDK (issue 2406), or
// Playwright, each attached to the Chrome that `xtask compare` launched,
// through the counting relay at `--cdp`, carries out the plan `--plan` names
// as `plan.ts` does and prints one JSON line of what each page came to. A
// Session is Collie's `run.new_session` with a `cdp_url` source, or
// Playwright's `browser.newContext()`; starting Collie's engine and
// Playwright's `connectOverCDP` stay outside the marks. Both Collie rows
// start the engine with `engine.ts`, which counts the messages sent it.
//
//   node act.ts --tool collie --plan plan.json \
//     --cdp ws://127.0.0.1:<relay>/devtools/browser/<id> \
//     --marks http://127.0.0.1:<relay>
import { type Page as CollieDriverPage, type Session as CollieSession } from "@goodfetch/collie";
import { connectOver } from "@goodfetch/collie/testing";
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
import { startEngine } from "./engine.ts";
import { raw } from "./raw.ts";

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

// Both Collie rows' engine reads the process's environment, which xtask
// set: the trial's Goodfetch home, whose user configuration maps the
// Proving ground's hosts, and the engine's path.

/**
 * Collie's TypeScript SDK, its Connection over the engine the driver
 * started: `connectOver` takes that engine's stdio, which counts the
 * messages sent it, where the SDK's own start would hide them; the program
 * leaves the engine's stderr to xtask, which the SDK's own start reads.
 */
async function collieProgram(cdp: string): Promise<Driver> {
  const engine = await startEngine();
  const connection = await connectOver(engine.transport);
  const run = await connection.start();
  return {
    newSession: async () =>
      collieSession(await run.newSession({ source: { kind: "cdp_url", url: cdp } })),
    async close() {
      await run.finish();
      // Closing the Connection ends the engine's input, which ends it.
      await connection.close();
      await engine.exited;
    },
    sent: () => engine.sent(),
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
  collie: async (_plan, cdp) => raw(await startEngine(), cdp),
  collie_program: (_plan, cdp) => collieProgram(cdp),
  playwright,
});
