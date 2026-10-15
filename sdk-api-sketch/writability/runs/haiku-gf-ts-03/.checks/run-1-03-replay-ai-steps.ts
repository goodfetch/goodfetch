// Scenario 03: the author lookup on quotes.toscrape.com, written as steps in plain words.
// The model resolves each step once and the Plan file next to this program keeps the result (ADR 0091).
// A later run replays the kept steps with no model call; a step whose target has drifted goes back to
// the model alone, and the program reports which step that was.

import { fileURLToPath } from "node:url";
import { Collie, expect, type ModelReference, type StepResult } from "@goodfetch/collie";
import { z } from "zod";

const SITE = "https://quotes.toscrape.com/";
const AUTHOR_PAGE_PREFIX = "https://quotes.toscrape.com/author/";
const MODEL: ModelReference = "vercel/anthropic/claude-sonnet-4.5";
const PLAN_FILE = fileURLToPath(new URL("./03-replay-ai-steps.plan.json", import.meta.url));
const RUN_TIMEOUT_MS = 10 * 60 * 1000;

const AuthorSchema = z.object({
  name: z.string(),
  birthDate: z.string(),
  birthplace: z.string(),
});

type Author = z.infer<typeof AuthorSchema>;

interface ResolvedStep {
  readonly name: string;
  readonly source: string;
  readonly instruction: string;
}

const resolvedSteps: ResolvedStep[] = [];

/** Records how a step was resolved, reports any step that went back to the model, and returns its output. */
function keepStep<T>(name: string, result: StepResult<T>): T {
  resolvedSteps.push({ name, source: result.step.source, instruction: result.step.instruction });
  if (result.step.source === "model" || result.step.source === "repair") {
    const drift = result.step.drift;
    const reason = drift ? ` after drift (expected ${drift.expected}, observed ${drift.observed ?? "nothing"})` : "";
    console.log(`Step "${name}" went back to the model${reason}: ${result.step.instruction}`);
  }
  return result.output;
}

async function lookUpAuthor(): Promise<Author> {
  await using collie = await Collie.connect();
  await using run = await collie.start({
    settings: { ai: { model: MODEL } },
    plan: PLAN_FILE,
    timeout: RUN_TIMEOUT_MS,
  });
  const page = await run.newPage();

  await page.goto(SITE);

  keepStep("open tag love", await page.act("In the Top Ten tags box, click the link named love"));
  await expect(page).toHaveURL(/^https:\/\/quotes\.toscrape\.com\/tag\/love\//);

  keepStep(
    "go to second page",
    await page.act("Follow the pagination link that leads to page 2 of this tag's quotes"),
  );
  await expect(page).toHaveURL(/^https:\/\/quotes\.toscrape\.com\/tag\/love\/page\/2\//);

  keepStep(
    "open first quote's author",
    await page.act("In the first quote on the page, click the about link beside the author's name"),
  );
  await expect(page).toHaveURL(/^https:\/\/quotes\.toscrape\.com\/author\//);

  // Confirmed in code, without a model: the page reached must be an author page.
  const reached = page.url();
  if (!reached.startsWith(AUTHOR_PAGE_PREFIX)) {
    throw new Error(`Expected an author page under ${AUTHOR_PAGE_PREFIX}, reached ${reached}`);
  }

  const author = keepStep(
    "read author details",
    await page.extract(
      "Read the author's full name, birth date and birthplace from this author page",
      AuthorSchema,
    ),
  );

  const report = await run.finish();
  console.log(`Run verdict: ${report.verdict}; plan: ${report.plan.status}`);

  const modelSteps = resolvedSteps.filter((step) => step.source === "model" || step.source === "repair");
  console.log(
    modelSteps.length === 0
      ? "No step called the model: every step replayed from the plan file."
      : `Steps that went back to the model: ${modelSteps.map((step) => step.name).join(", ")}`,
  );

  return author;
}

const author = await lookUpAuthor();
console.log(JSON.stringify(author, null, 2));
