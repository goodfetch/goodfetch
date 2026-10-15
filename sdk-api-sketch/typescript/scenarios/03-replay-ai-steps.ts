/// <reference types="node" />
// Golden scenario 03, "Repeat AI steps with no model call" (ADRs 0091, 0101, 0102).
import { z } from "zod";
import { Collie, expect } from "@goodfetch/collie";

const Author = z.object({ name: z.string(), birthDate: z.string(), birthplace: z.string() });

// The model is called only for a step the Plan file lacks or a step that drifted (ADR 0091).
await using collie = await Collie.connect({
  settings: { ai: { model: "anthropic/claude-sonnet-4-5" } },
});
// One entry per call, kept next to the program; a new version is written only after a `verified` Run.
await using run = await collie.start({ plan: `${import.meta.dirname}/quotes.plan.json` });
const page = await run.newPage();
await page.goto("https://quotes.toscrape.com/");
for (const instruction of [
  'open the tag "love" from the tag list',
  "go to its second page",
  "open the about page of the first quote's author",
]) {
  // Replayed with no model call when its Target fingerprint still matches; on Drift, this step alone
  // gets one Repair, and a Drift left (no model, `plan.frozen`) throws PlanDriftError naming it.
  const { step } = await page.act(instruction);
  if (step.source === "repair")
    console.warn(`The site changed: "${step.instruction}" went back to the model.`);
}
// A Success check without a model: the Run is `verified`, and its Plan kept, only if it holds.
await expect(page).toHaveURL(/^https:\/\/quotes\.toscrape\.com\/author\//);
// Compiled by default: the model writes each field's locator once, then values are copied from the page.
const { output: author } = await page.extract(
  "the author's name, birth date and birthplace",
  Author,
);
const report = await run.finish();
console.log(author, `Plan ${report.plan.status}`, report.plan.changedSteps);
