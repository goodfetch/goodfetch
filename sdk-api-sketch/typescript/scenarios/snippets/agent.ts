// Snippet for the Agent in Collie's SDK API (#474, ADR 0100), on golden scenario 11.
import { z } from "zod";
import {
  AgentToolFailedError,
  Collie,
  GoodfetchError,
  agentTool,
  expect,
  type AgentResult,
  type Logger,
  type Page,
  type Run,
} from "@goodfetch/collie";

declare const log: Logger; // the program's own logger

const Answer = z.object({
  title: z.string(),
  price: z.string().regex(/^£\d+\.\d{2}$/),
});
type Answer = z.infer<typeof Answer>;

// Golden scenario 11, "Reach a goal from one instruction, with limits and a verdict". The Run caps the
// price at 0.20 USD (a Limit, which stops the Run), the task caps itself at 25 steps (which stops only
// the task), and the program, not the model, states what success looks like. With `plan`, the steps
// are kept, and a later run repeats them with no model call.
export async function cheapestFiveStarTravelBook(): Promise<Answer | undefined> {
  await using collie = await Collie.connect({ logger: log });
  await using run = await collie.start({
    plan: "travel.plan.json",
    limits: { price: { amount: "0.20", currency: "USD" } },
  });
  const page = await run.newPage();
  await page.goto("https://books.toscrape.com/");

  const task = page.agent("find the cheapest book rated five stars in the Travel category and give its title and price", {
    schema: Answer,
    maxSteps: 25,
    // By default the task stays on toscrape.com, where it starts; this narrows it to one host.
    domains: ["books.toscrape.com"],
    // The Success check: a book page in Travel with five stars. A model's "done" is only a claim.
    successCheck: async (page) => {
      await expect(page.locator(".breadcrumb")).toContainText("Travel");
      await expect(page.locator(".product_main .star-rating")).toHaveClass(/\bFive\b/);
    },
  });

  // The steps as they happen, from step 1, none dropped.
  for await (const step of task.steps()) {
    log.info(`step ${step.index} (${step.origin})`, { url: step.url, did: step.actions.map((a) => a.code ?? a.operation) });
  }

  const result = await task;
  log.info("agent done", { tokens: result.usage.tokens, price: result.usage.price, verdict: result.verdict });
  return report(result);
}

// How the work ended, its Verdict and the model's claim are three separate fields (ADR 0079).
function report(result: AgentResult<Answer>): Answer | undefined {
  if (result.ended.kind === "stopped") {
    const reason = result.ended.reason;
    // `max_steps`, `no_progress` and `timeout` name the setting that stopped it; `limit` names the Limit;
    // `gave_up` is the model giving up, its message in `reported`. The Grounded partial answer is beside the reason.
    log.warn("agent stopped", { reason: reason.kind, partial: result.ended.partial, claimed: result.reported?.claim });
    return undefined;
  }
  if (result.verdict !== "verified") {
    // `rejected`: the Success check failed after the model said done; `needs-review`: nothing proved it.
    log.warn(`agent ${result.verdict}`, { claimed: result.reported?.claim, message: result.reported?.message });
    return undefined;
  }
  return result.requireVerified();
}

// An Agent tool: the program's own function as an action. Read-only, so it is no Effect.
declare const shelf: { has(title: string): Promise<boolean> }; // the program's own data
const alreadyOwned = agentTool({
  name: "already_owned",
  description: "Whether the library already owns a book with this exact title",
  input: z.object({ title: z.string() }),
  readOnly: true,
  run: async ({ title }) => ({ owned: await shelf.has(title) }),
});

// A task awaited in one line, with a tool. `requireVerified()` throws `NotVerifiedError`, with every
// reason, when the Verdict is not `verified`.
export async function cheapestUnownedBook(run: Run): Promise<Answer> {
  const page: Page = await run.newPage();
  await page.goto("https://books.toscrape.com/catalogue/category/books/travel_2/index.html");
  try {
    const result = await page.agent("find the cheapest five-star book here that the library does not own yet", {
      schema: Answer,
      tools: [alreadyOwned],
      successCheck: async (page) => {
        await expect(page.locator(".product_main .star-rating")).toHaveClass(/\bFive\b/);
      },
    });
    // Each page action carries its code in this language, to turn the task into plain page API calls.
    log.debug("as code", { code: result.steps.flatMap((s) => s.actions.flatMap((a) => (a.code ? [a.code] : []))) });
    return result.requireVerified();
  } catch (error) {
    if (error instanceof AgentToolFailedError) log.error(`tool ${error.details.tool} threw`, { message: error.details.exceptionMessage });
    // Whatever ended the task, the steps taken and the Grounded partial answer travel on the error.
    if (error instanceof GoodfetchError) log.warn("partial answer", { partial: error.agentTask?.partial });
    throw error;
  }
}
