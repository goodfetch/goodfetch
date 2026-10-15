// Opens the tag "love" on quotes.toscrape.com, goes to its second page and opens the author page of
// the first quote, all with steps written in plain words. The Run's Plan file, next to this program,
// keeps what the model resolved for each step: the first run compiles them, later runs replay them
// with no model call, and a step whose kept entry no longer fits goes back to the model alone (one
// Repair), which the program reports by its instruction.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { Collie, expect, type ModelReference, type StepResult } from "@goodfetch/collie";
import { z } from "zod";

const here = path.dirname(fileURLToPath(import.meta.url));
// The Plan file, next to the program; the Run takes its path relative to the working directory.
const planFile = path.relative(process.cwd(), path.join(here, "03-replay-ai-steps.plan.json"));

// Only used by a step the Plan does not hold (first run) or one that drifted; a replay never calls it.
const model = (process.env["GOODFETCH_AI_MODEL"] ?? "anthropic/claude-sonnet-4-5") as ModelReference;

const authorSchema = z.object({
  name: z.string().describe("The author's full name, from the heading of the author page"),
  birthDate: z.string().describe("The author's birth date, as written after 'Born:'"),
  birthplace: z.string().describe("The author's birthplace, as written after 'in' in the birth line"),
});

/** The steps the model resolved or repaired in this Run, by their instruction. */
const resolvedByModel: { instruction: string; source: string }[] = [];

function note<T>(result: StepResult<T>): T {
  const { source, instruction, drift } = result.step;
  if (source === "model" || source === "repair") {
    resolvedByModel.push({ instruction, source });
    if (source === "repair") {
      console.log(
        `Step went back to the model (kept entry no longer applied): "${instruction}"` +
          (drift ? ` (expected ${drift.expected}, observed ${drift.observed ?? "nothing"})` : ""),
      );
    } else {
      console.log(`Step compiled by the model (not in the Plan yet): "${instruction}"`);
    }
  }
  return result.output;
}

await using collie = await Collie.connect({ settings: { ai: { model } } });
await using run = await collie.start({ plan: planFile });
const page = await run.newPage();

await page.goto("https://quotes.toscrape.com/");

note(await page.act('Click the tag "love" in the list of popular tags on the right'));
note(await page.act("Go to the second page of the quotes by clicking the Next link"));
note(await page.act('Click the "(about)" link next to the author of the first quote'));

// A check with no model: the page reached must be an author page.
await expect(page).toHaveURL(/^https:\/\/quotes\.toscrape\.com\/author\//);

const author = note(
  await page.extract("Read the author's name, birth date and birthplace from this author page", authorSchema),
);

const report = await run.finish();

if (resolvedByModel.length === 0) {
  console.log("Every step was replayed from the Plan: no model call.");
} else {
  console.log(`Steps resolved by the model in this run: ${resolvedByModel.map((s) => `"${s.instruction}" (${s.source})`).join("; ")}`);
}
console.log(`Plan ${report.plan.file ?? "(none)"}: ${report.plan.status}; changed steps: ${JSON.stringify(report.plan.changedSteps)}`);
console.log(JSON.stringify(author, null, 2));
