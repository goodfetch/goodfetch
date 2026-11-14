// One trial of the Collie scenario of `just compare` with one of the arms
// (issues 2443, 2441 and 2442): Puppeteer, through its locators or its page
// methods, Stagehand with no model, or agent-browser's command line, each
// attached to the Chrome that `xtask compare` launched, through the counting
// relay at `--cdp`. It carries out the plan `--plan` names as
// `../compare-collie/plan.ts` does and prints one JSON line of what each
// page came to.
//
// A Session is Puppeteer's `browser.createBrowserContext()`. Stagehand
// drives the browser's one context, and agent-browser has no command that
// closes a context it opened, so neither opens a Session of its own and
// xtask plans them no Sessions cell. Connecting stays outside the marks.
//
//   node act.ts --tool puppeteer_locator --plan plan.json \
//     --cdp ws://127.0.0.1:<relay>/devtools/browser/<id> \
//     --marks http://127.0.0.1:<relay>
import {
  localBrowser,
  Stagehand,
  type Page as StagehandDriverPage,
} from "@browserbasehq/stagehand";
import { findNativeBinary, nativeTarget } from "agent-browser/scripts/native.js";
import { execFile, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import process from "node:process";
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

/** What one agent-browser command answered, from its `--json` line. */
interface AgentBrowserAnswer {
  readonly success: boolean;
  readonly data: Readonly<Record<string, unknown>> | null;
  readonly error: string | null;
}

function isAgentBrowserAnswer(value: unknown): value is AgentBrowserAnswer {
  if (typeof value !== "object" || value === null) return false;
  const answer: Record<string, unknown> = { ...value };
  const data = answer["data"];
  const error = answer["error"];
  return (
    typeof answer["success"] === "boolean" &&
    (data === null || data === undefined || (typeof data === "object" && !Array.isArray(data))) &&
    (error === null || error === undefined || typeof error === "string")
  );
}

/** The `data` of a command that succeeded; the command's error otherwise. */
function agentBrowserData(stdout: string, stderr: string): Readonly<Record<string, unknown>> {
  let parsed: unknown;
  try {
    parsed = JSON.parse(stdout);
  } catch {
    throw new Error(`agent-browser answered no JSON: ${stderr.trim() || stdout.trim()}`);
  }
  if (!isAgentBrowserAnswer(parsed)) throw new Error(`agent-browser answered ${stdout.trim()}`);
  if (!parsed.success) throw new Error(parsed.error ?? "agent-browser failed with no error");
  return parsed.data ?? {};
}

/** An agent-browser Session's close: its daemons close with the driver. */
async function leaveInstancesOpen(): Promise<void> {}

/** The native binary agent-browser's own launcher would run on this host. */
function agentBrowserBinary(): string {
  const root = dirname(createRequire(import.meta.url).resolve("agent-browser/package.json"));
  const target = nativeTarget();
  const binary = target === null ? null : findNativeBinary(root, target);
  if (binary === null) throw new Error("agent-browser has no native binary for this host");
  return binary;
}

/**
 * agent-browser's command line. Each command is a process that hands the
 * command to the daemon of its `--session`, an instance of the tool that the
 * first command starts and that stays attached to the browser. Pages one
 * after another share one instance; pages at once each get their own,
 * started before the start mark, and each command pins its instance to the
 * page it opened (`--pin-tab`), so no instance follows another's new page. A
 * click checks once that its target can take it and does not wait.
 */
async function agentBrowser(plan: Plan, cdp: string): Promise<Driver> {
  const binary = agentBrowserBinary();
  // A short folder: a daemon's socket path must fit the OS's limit. The
  // empty config keeps the user's and the folder's agent-browser.json out.
  const folder = mkdtempSync("/tmp/ab-");
  const config = join(folder, "config.json");
  writeFileSync(config, "{}");
  const env = {
    ...process.env,
    AGENT_BROWSER_SOCKET_DIR: folder,
    AGENT_BROWSER_CONFIG: config,
    AGENT_BROWSER_NO_VERSION_CHECK: "1",
    AGENT_BROWSER_DEFAULT_TIMEOUT: String(plan.action_timeout),
    // A daemon a crashed trial left behind ends on its own.
    AGENT_BROWSER_IDLE_TIMEOUT_MS: "60000",
  };
  const started: string[] = [];
  const argv = (instance: string, args: readonly string[]): string[] => [
    "--session",
    instance,
    "--cdp",
    cdp,
    "--json",
    "--pin-tab",
    ...args,
  ];
  const run = (instance: string, args: readonly string[]) =>
    new Promise<Readonly<Record<string, unknown>>>((resolve, reject) => {
      execFile(binary, argv(instance, args), { env }, (_failed, stdout, stderr) => {
        // A command that fails exits 1 and still prints its answer, which
        // names the error.
        try {
          resolve(agentBrowserData(stdout, stderr));
        } catch (error) {
          reject(error);
        }
      });
    });
  const start = async (instance: string) => {
    started.push(instance);
    await run(instance, ["connect", cdp]);
  };
  const closeAll = () => {
    for (const instance of started.splice(0)) {
      spawnSync(binary, argv(instance, ["close"]), { env, stdio: "ignore" });
    }
    rmSync(folder, { recursive: true, force: true });
  };
  // A trial that fails before the driver's close leaves no daemon behind.
  process.once("exit", closeAll);

  const page = (instance: string, pageId: string): DriverPage => ({
    async goto(url) {
      await run(instance, ["open", url]);
      // `open` answers no status: the navigation entry has it.
      const data = await run(instance, [
        "eval",
        "performance.getEntriesByType('navigation')[0]?.responseStatus",
      ]);
      const status = data["result"];
      return typeof status === "number" && status > 0 ? status : null;
    },
    async step(step) {
      if (step.action === "read") return readText((await run(instance, ["eval", READ]))["result"]);
      if (step.action === "click") await run(instance, ["click", step.selector]);
      else if (step.action === "fill")
        await run(instance, ["fill", step.selector, "--", step.value]);
      else {
        await run(instance, ["focus", step.selector]);
        await run(instance, ["press", step.key]);
      }
      return null;
    },
    async close() {
      await run(instance, ["tab", "close", pageId]);
    },
  });
  const newPage = async (instance: string): Promise<DriverPage> => {
    const pageId = (await run(instance, ["tab", "new"]))["tabId"];
    if (typeof pageId !== "string") throw new Error("agent-browser's `tab new` named no page");
    return page(instance, pageId);
  };

  let opened = false;
  return {
    async newSession() {
      if (opened) throw new Error("agent-browser has no command that closes a context it opened");
      opened = true;
      if (!plan.at_once) {
        await start("s");
        return { newPage: () => newPage("s"), close: leaveInstancesOpen };
      }
      const instances = plan.pages.map((_, index) => `p${index}`);
      for (const instance of instances) await start(instance);
      let next = 0;
      return {
        async newPage() {
          const instance = instances[next];
          next += 1;
          if (instance === undefined)
            throw new Error("the plan opens more pages at once than it lists");
          return newPage(instance);
        },
        close: leaveInstancesOpen,
      };
    },
    async close() {
      process.removeListener("exit", closeAll);
      closeAll();
    },
  };
}

await main({
  puppeteer_locator: (plan, cdp) => puppeteerDriver(plan, cdp, throughLocators),
  puppeteer_page: (plan, cdp) => puppeteerDriver(plan, cdp, throughPageMethods),
  stagehand,
  agent_browser: agentBrowser,
});
