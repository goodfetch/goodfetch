// The plan of a Collie scenario trial of `just compare` (issue 2282), and
// the loop that carries it out with one tool: Collie's and Playwright's in
// `act.ts` beside this file, the arms' in `xtask/compare-collie-arms/act.ts`
// (issues 2443 and 2441). xtask writes the plan; a driver only carries it
// out, the same steps for every tool:
//
// - each page of `pages` opens in the trial's one Session, goes to its URL
//   and takes its steps, then closes; one after another, or all at once
//   when `at_once` is set;
// - with `sessions` set, that many Sessions open one after another, each
//   with one page, and close;
// - with `openings` set, that many pages open one after another in the
//   trial's one Session, each kept open until the last is (issue 2319).
//
// Around the measured part it asks `--marks` for `/start` and `/end`, which
// xtask answers once it has read the relay's command count and the
// process tree's CPU time. Setting up and tearing down stay outside the
// marks. It imports only Node.js's own modules, so both sides' packages
// can import it.
import { readFileSync } from "node:fs";
import process from "node:process";
import { parseArgs } from "node:util";

/** One step on a page. */
export type Step =
  | { readonly action: "click"; readonly selector: string }
  | { readonly action: "fill"; readonly selector: string; readonly value: string }
  | { readonly action: "press"; readonly selector: string; readonly key: string }
  | { readonly action: "read" };

/** One page of the plan: a Proving ground task's page and its steps. */
export interface PlannedPage {
  readonly task: string;
  readonly url: string;
  readonly steps: readonly Step[];
}

/** What xtask asks of the trial. */
export interface Plan {
  readonly pages: readonly PlannedPage[];
  readonly at_once: boolean;
  readonly sessions: number;
  /** Pages opened one after another in one Session, each kept open. */
  readonly openings: number;
  /** How long an action waits for its target, in milliseconds. */
  readonly action_timeout: number;
}

/** What one page came to. */
export interface PageResult {
  readonly task: string;
  /** Whether every step succeeded, as the tool reported it. */
  readonly outcome: "success" | "failure";
  /** The document's status, when the navigation answered one. */
  readonly status: number | null;
  /** The steps it took, the one that failed included. */
  readonly steps: number;
  /** The text a `read` step read, when the plan had one. */
  readonly read: string | null;
  /** The first step's error, when one failed. */
  readonly error: string | null;
}

/** A page of any tool. */
export interface DriverPage {
  goto(url: string): Promise<number | null>;
  step(step: Step): Promise<string | null>;
  close(): Promise<void>;
}

/** A Session of any tool. */
export interface DriverSession {
  newPage(): Promise<DriverPage>;
  close(): Promise<void>;
}

/** A tool, attached to the browser. */
export interface Driver {
  newSession(): Promise<DriverSession>;
  close(): Promise<void>;
}

// An expression, which every tool evaluates as it is: Playwright calls only
// a function it is handed as one, not a function's source. The title, which
// the served page sets, where the body's text would race the scripts that
// fill some portal pages after their load.
export const READ = "document.title";

/** The text a read step read, or none when the page answered no string. */
export function readText(value: unknown): string {
  return typeof value === "string" ? value : "";
}

/** Opens a page of `session` on `planned`, takes its steps and closes it. */
async function onPage(session: DriverSession, planned: PlannedPage): Promise<PageResult> {
  const page = await session.newPage();
  let status: number | null = null;
  let read: string | null = null;
  let steps = 0;
  try {
    status = await page.goto(planned.url);
    for (const step of planned.steps) {
      steps += 1;
      const text = await page.step(step);
      if (text !== null) read = text;
    }
    return { task: planned.task, outcome: "success", status, steps, read, error: null };
  } catch (error) {
    const message = error instanceof Error ? (error.message.split("\n")[0] ?? "") : String(error);
    return { task: planned.task, outcome: "failure", status, steps, read, error: message };
  } finally {
    await page.close();
  }
}

/** The plan xtask wrote, its shape checked. */
function parsePlan(text: string): Plan {
  const parsed: unknown = JSON.parse(text);
  if (!isPlan(parsed)) throw new Error("the plan is not the shape xtask writes");
  return parsed;
}

function isPlan(value: unknown): value is Plan {
  if (typeof value !== "object" || value === null) return false;
  const plan: Record<string, unknown> = { ...value };
  return (
    Array.isArray(plan["pages"]) &&
    plan["pages"].every(isPlannedPage) &&
    typeof plan["at_once"] === "boolean" &&
    typeof plan["sessions"] === "number" &&
    typeof plan["openings"] === "number" &&
    typeof plan["action_timeout"] === "number"
  );
}

function isPlannedPage(value: unknown): value is PlannedPage {
  if (typeof value !== "object" || value === null) return false;
  const page: Record<string, unknown> = { ...value };
  return (
    typeof page["task"] === "string" &&
    typeof page["url"] === "string" &&
    Array.isArray(page["steps"]) &&
    page["steps"].every(isStep)
  );
}

function isStep(value: unknown): value is Step {
  if (typeof value !== "object" || value === null) return false;
  const step: Record<string, unknown> = { ...value };
  switch (step["action"]) {
    case "read":
      return true;
    case "click":
      return typeof step["selector"] === "string";
    case "fill":
      return typeof step["selector"] === "string" && typeof step["value"] === "string";
    case "press":
      return typeof step["selector"] === "string" && typeof step["key"] === "string";
    default:
      return false;
  }
}

async function mark(marks: string, which: "start" | "end"): Promise<void> {
  const answer = await fetch(`${marks}/${which}`);
  if (!answer.ok) throw new Error(`the ${which} mark answered ${answer.status}`);
  await answer.arrayBuffer();
}

/** Opens each tool its driver: the plan, and the endpoint at `cdp`. */
export type Drivers = Readonly<Record<string, (plan: Plan, cdp: string) => Promise<Driver>>>;

/**
 * Carries out the plan `--plan` names with the tool `--tool` names, one of
 * `drivers`, and prints one JSON line of what each page came to.
 */
export async function main(drivers: Drivers): Promise<void> {
  const { values } = parseArgs({
    options: {
      tool: { type: "string" },
      plan: { type: "string" },
      cdp: { type: "string" },
      marks: { type: "string" },
    },
    strict: true,
  });
  const open =
    values.tool !== undefined && Object.hasOwn(drivers, values.tool)
      ? drivers[values.tool]
      : undefined;
  if (
    open === undefined ||
    values.plan === undefined ||
    values.cdp === undefined ||
    values.marks === undefined
  ) {
    process.stderr.write(
      `usage: node act.ts --tool <${Object.keys(drivers).join("|")}> --plan <plan.json> --cdp <ws url> --marks <http url>\n`,
    );
    process.exit(2);
  }
  const plan = parsePlan(readFileSync(values.plan, "utf8"));
  const driver = await open(plan, values.cdp);
  let pages: PageResult[] = [];
  let sessions = 0;
  let opened = 0;
  if (plan.openings > 0) {
    // One page first, outside the marks, as the Sessions cell's first
    // Session; the measured pages stay open until the end mark.
    const session = await driver.newSession();
    const first = await session.newPage();
    await mark(values.marks, "start");
    const kept: DriverPage[] = [];
    while (opened < plan.openings) {
      kept.push(await session.newPage());
      opened += 1;
    }
    await mark(values.marks, "end");
    for (const page of kept) await page.close();
    await first.close();
    await session.close();
  } else if (plan.sessions > 0) {
    // One Session first, outside the marks, so each tool is attached to the
    // browser before the openings it measures.
    const first = await driver.newSession();
    await mark(values.marks, "start");
    for (let started = 0; started < plan.sessions; started += 1) {
      const session = await driver.newSession();
      const page = await session.newPage();
      await page.close();
      await session.close();
      sessions += 1;
    }
    await mark(values.marks, "end");
    await first.close();
  } else {
    const session = await driver.newSession();
    await mark(values.marks, "start");
    if (plan.at_once) {
      pages = await Promise.all(plan.pages.map((planned) => onPage(session, planned)));
    } else {
      for (const planned of plan.pages) pages.push(await onPage(session, planned));
    }
    await mark(values.marks, "end");
    await session.close();
  }
  await driver.close();
  process.stdout.write(`${JSON.stringify({ pages, sessions, opened })}\n`);
}
