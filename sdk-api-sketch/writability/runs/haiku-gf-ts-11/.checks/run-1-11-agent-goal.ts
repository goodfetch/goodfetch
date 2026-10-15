// One instruction on books.toscrape.com, run as one Agent task: the model finds the cheapest five-star
// Travel book and gives its title and price. The task is capped at 25 steps and 0.20 USD, bounded to the
// site's domain, verified by a Success check the program states, and kept as a Plan entry so the same
// steps replay without a model.
import { Collie, GoodfetchError, expect } from "@goodfetch/collie";
import type { AgentStep, Ended, Money, ModelReference, ModelUsage, Page, Reported, RunReport, Verdict } from "@goodfetch/collie";
import { z } from "zod";

const SITE = "books.toscrape.com";
const START_URL = `https://${SITE}/`;
const INSTRUCTION = "find the cheapest book rated five stars in the Travel category and give its title and price";
const MODEL: ModelReference = "anthropic/claude-haiku-4-5";
const MAX_STEPS = 25;
const PRICE_CAP: Money = { amount: "0.20", currency: "USD" };
const RUN_TIMEOUT_MS = 15 * 60_000;
const TASK_TIMEOUT_MS = 10 * 60_000;
const PLAN_FILE = "11-agent-goal.plan.json";
const BOOK_PAGE_URL = /^https:\/\/books\.toscrape\.com\/catalogue\/.+_\d+\/index\.html$/;

// The typed fields the task returns.
const AnswerSchema = z.object({
  title: z.string().describe("The book's title, exactly as its page shows it"),
  price: z.string().describe("The book's price, exactly as its page shows it, with the currency sign"),
});

// The Success check: the page the task ends on is a book page in the Travel category rated five stars.
// The model's "done" is only a signal; the Verdict reads this check.
async function onFiveStarTravelBook(page: Page): Promise<void> {
  await expect(page).toHaveURL(BOOK_PAGE_URL);
  await expect(page.locator('ul.breadcrumb a[href*="/category/books/travel_2/"]')).toBeVisible();
  await expect(page.locator("p.star-rating.Five")).toBeVisible();
}

interface Ending {
  readonly how: "verified" | "stopped" | "reported_done" | "not_verified" | "error";
  readonly detail: string;
}

// How the work ended. A verified Verdict comes first, then a stop (a limit, the step cap, a timeout), then
// the model's own claim of being done, which is a signal and never proof.
function endingOf(result: { readonly ended: Ended; readonly verdict: Verdict; readonly reported: Reported | undefined }): Ending {
  if (result.verdict === "verified") {
    return { how: "verified", detail: "the Success check held: a book page in Travel with a five-star rating" };
  }
  if (result.ended.kind === "stopped") {
    const reason = result.ended.reason;
    return { how: "stopped", detail: reason.kind === "limit" ? `limit ${reason.limit}` : reason.kind };
  }
  if (result.reported?.claim === "done") {
    return { how: "reported_done", detail: result.reported.message };
  }
  return { how: "not_verified", detail: `verdict ${result.verdict}` };
}

interface StepRecord {
  readonly index: number;
  readonly origin: string;
  readonly url: string;
  readonly actions: readonly string[];
}

// The page API code of each action, so the steps can be read and repeated without a model.
function stepRecord(step: AgentStep): StepRecord {
  return {
    index: step.index,
    origin: step.origin,
    url: step.url,
    actions: step.actions.map((action) => action.code ?? action.operation),
  };
}

interface UsageRecord {
  readonly calls: number;
  readonly tokens: number;
  readonly price: Money | null;
}

function usageRecord(usage: ModelUsage): UsageRecord {
  return { calls: usage.calls, tokens: usage.tokens, price: usage.price };
}

interface TaskSummary {
  readonly title: string | null;
  readonly price: string | null;
  readonly ending: Ending;
  readonly steps: readonly StepRecord[];
  readonly usage: UsageRecord | null;
}

// A failure the Agent throws is an ending, with the steps it took before it.
function failureOf(error: unknown): TaskSummary {
  if (error instanceof GoodfetchError) {
    const steps: readonly AgentStep[] = error.agentTask?.steps ?? [];
    return {
      title: null,
      price: null,
      ending: { how: "error", detail: `${error.code}: ${error.message}` },
      steps: steps.map(stepRecord),
      usage: error.usage ? usageRecord(error.usage) : null,
    };
  }
  return {
    title: null,
    price: null,
    ending: { how: "error", detail: error instanceof Error ? error.message : String(error) },
    steps: [],
    usage: null,
  };
}

// One Agent task from one instruction, with the output as typed fields.
async function runTask(page: Page): Promise<TaskSummary> {
  try {
    const result = await page.agent(INSTRUCTION, {
      schema: AnswerSchema,
      successCheck: onFiveStarTravelBook,
      maxSteps: MAX_STEPS,
      domains: [SITE],
      timeout: TASK_TIMEOUT_MS,
    });
    return {
      title: result.output?.title ?? null,
      price: result.output?.price ?? null,
      ending: endingOf(result),
      steps: result.steps.map(stepRecord),
      usage: usageRecord(result.usage),
    };
  } catch (error: unknown) {
    return failureOf(error);
  }
}

interface RunRecord {
  readonly lifecycle: string;
  readonly verdict: string;
  readonly plan: string;
  readonly planFile: string | null;
}

function runRecord(report: RunReport): RunRecord {
  return { lifecycle: report.lifecycle, verdict: report.verdict, plan: report.plan.status, planFile: report.plan.file };
}

async function main(): Promise<void> {
  // The engine is the local Attached one, so nothing reaches a Daemon or a cloud service. Politeness is
  // kept to this run, so no shared store leaves the machine.
  await using collie = await Collie.connect({
    engine: "attached",
    settings: { politeness: { sharing: "run" } },
  });

  // The Limit ends the Run if the price cap is reached; the model allowance refuses any model call that
  // would exceed it. The Plan file keeps the task as one entry, replayed whole when it is run again.
  await using run = await collie.start({
    limits: { price: PRICE_CAP },
    plan: PLAN_FILE,
    timeout: RUN_TIMEOUT_MS,
    settings: {
      ai: { model: MODEL, allowance: { price: PRICE_CAP } },
      agent: { maxSteps: MAX_STEPS, domains: [SITE] },
    },
  });

  const page = await run.newPage();
  await page.goto(START_URL);

  const task = await runTask(page);
  const report = await run.finish();

  const summary = { ...task, run: runRecord(report) };
  console.log(JSON.stringify(summary, null, 2));
  if (task.ending.how === "error") {
    process.exitCode = 1;
  }
}

main().catch((error: unknown) => {
  console.error(error);
  process.exitCode = 1;
});
