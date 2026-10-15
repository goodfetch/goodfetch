// Golden scenario 11, "Reach a goal from one instruction, with limits and a verdict" (ADRs 0013, 0079, 0091, 0100, 0101).
import { z } from "zod";
import { Collie, expect, type AgentResult, type StopReason } from "@goodfetch/collie";

const Answer = z.object({ title: z.string(), price: z.string().regex(/^£\d+\.\d{2}$/) });
type Answer = z.infer<typeof Answer>;

/** How the work ended, with what it found. */
type Outcome =
  | { readonly ended: "verified"; readonly answer: Answer }
  | { readonly ended: "reported"; readonly claim: string; readonly answer: Answer }
  | { readonly ended: "stopped"; readonly reason: StopReason; readonly partial: Partial<Answer> };

// No telemetry exists to switch off: only the site and the model's provider are reached (ADR 0043).
await using collie = await Collie.connect(); // Anthropic's key: env:ANTHROPIC_API_KEY, sent only to Anthropic
await using run = await collie.start({
  // The task is one Plan entry: once a Run is verified, a later one replays it with no model call.
  plan: "travel.plan.json",
  limits: { price: { amount: "0.20", currency: "USD" } }, // a Limit: reaching it stops the Run too
});
const page = await run.newPage();
await page.goto("https://books.toscrape.com/");
const result = await page.agent(
  "find the cheapest book rated five stars in the Travel category and give its title and price",
  {
    schema: Answer,
    model: "anthropic/claude-sonnet-4-5",
    maxSteps: 25, // stops only the task
    // Enforced by the engine on every navigation the task makes, whatever the pages say (ADRs 0014, 0100).
    domains: ["books.toscrape.com"],
    // The Success check: the model's "done" is only a claim until this holds on the page it reached.
    successCheck: async (page) => {
      await expect(page.locator(".breadcrumb")).toContainText("Travel");
      await expect(page.locator(".product_main .star-rating")).toHaveClass(/\bFive\b/);
    },
  },
);
await run.finish();
console.log(outcome(result));

// The answer sits in `ended`: `finished` holds the output, `stopped` its reason and the Grounded partial.
function outcome(result: AgentResult<Answer>): Outcome {
  const ended = result.ended;
  if (ended.kind === "stopped") return { ended: "stopped", reason: ended.reason, partial: ended.partial };
  if (result.verdict === "verified") return { ended: "verified", answer: result.requireVerified() };
  return { ended: "reported", claim: result.reported?.claim ?? "none", answer: ended.output };
}
