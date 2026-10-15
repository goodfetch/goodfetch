import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { Stagehand, localBrowser, type Action, type Page } from "@browserbasehq/stagehand";
import { z } from "zod";

const SITE = "https://quotes.toscrape.com/";
const AUTHOR_PREFIX = "https://quotes.toscrape.com/author/";
const MODEL = "openai/gpt-4.1-mini";
const STEPS_FILE = fileURLToPath(new URL("./03-replay-ai-steps.steps.json", import.meta.url));

interface StepSpec {
  id: string;
  instruction: string;
  expectedUrlPrefix: string;
}

// Each step is written in plain words. The model resolves it once; the
// resolved actions are kept in STEPS_FILE and replayed on later runs.
const STEP_SPECS: readonly StepSpec[] = [
  {
    id: "open-love-tag",
    instruction: 'Click the tag "love" in the list of tags.',
    expectedUrlPrefix: `${SITE}tag/love/`,
  },
  {
    id: "go-to-second-page",
    instruction: "Open the second page of this tag's quotes.",
    expectedUrlPrefix: `${SITE}tag/love/page/2/`,
  },
  {
    id: "open-first-author-about",
    instruction: 'Click the "(about)" link of the author of the first quote on this page.',
    expectedUrlPrefix: AUTHOR_PREFIX,
  },
];

const ActionSchema = z.object({
  selector: z.string(),
  description: z.string(),
  method: z.string().optional(),
  arguments: z.array(z.string()).optional(),
});

const StepFileSchema = z.object({
  version: z.literal(1),
  site: z.string(),
  steps: z.array(
    z.object({
      id: z.string(),
      instruction: z.string(),
      actions: z.array(ActionSchema),
    }),
  ),
});

type StepFile = z.infer<typeof StepFileSchema>;

interface AuthorDetails {
  name: string;
  birthDate: string;
  birthplace: string;
}

type ReplayOutcome = { ok: true } | { ok: false; reason: string };

async function loadKeptSteps(): Promise<Map<string, Action[]>> {
  let text: string;
  try {
    text = await readFile(STEPS_FILE, "utf8");
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") {
      return new Map();
    }
    throw error;
  }

  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    console.log(`The kept steps file is not valid JSON; every step goes to the model.`);
    return new Map();
  }

  const parsed = StepFileSchema.safeParse(raw);
  if (!parsed.success) {
    console.log(`The kept steps file has an unexpected shape; every step goes to the model.`);
    return new Map();
  }

  return new Map(
    parsed.data.steps.map((step): [string, Action[]] => [step.id, step.actions]),
  );
}

async function saveKeptSteps(actionsById: ReadonlyMap<string, Action[]>): Promise<void> {
  const file: StepFile = {
    version: 1,
    site: SITE,
    steps: STEP_SPECS.map((spec) => ({
      id: spec.id,
      instruction: spec.instruction,
      actions: actionsById.get(spec.id) ?? [],
    })),
  };
  await writeFile(STEPS_FILE, `${JSON.stringify(file, null, 2)}\n`, "utf8");
}

async function activePage(stagehand: Stagehand): Promise<Page> {
  const page = await stagehand.browser.context.activePage();
  if (page === undefined) {
    throw new Error("The browser has no active page.");
  }
  return page;
}

// Replays kept actions with no model call: selfHeal is off and act() receives
// the stored Action objects, not an instruction.
async function replayStep(
  stagehand: Stagehand,
  page: Page,
  spec: StepSpec,
  actions: readonly Action[],
): Promise<ReplayOutcome> {
  if (actions.length === 0) {
    return { ok: false, reason: "no kept action" };
  }
  for (const action of actions) {
    try {
      const result = await stagehand.act(action);
      if (!result.data.success) {
        return { ok: false, reason: result.data.message };
      }
    } catch (error) {
      return { ok: false, reason: error instanceof Error ? error.message : String(error) };
    }
  }
  await page.waitForLoadState("load");
  const url = await page.url();
  if (!url.startsWith(spec.expectedUrlPrefix)) {
    return { ok: false, reason: `the page is at ${url}` };
  }
  return { ok: true };
}

// Only called for a step with no kept resolution, or whose kept actions no
// longer apply. Returns the actions the model resolved, to be kept.
async function resolveWithModel(stagehand: Stagehand, page: Page, spec: StepSpec): Promise<Action[]> {
  const result = await stagehand.act(spec.instruction);
  if (!result.data.success || result.data.actions.length === 0) {
    throw new Error(`The model could not resolve step "${spec.id}": ${result.data.message}`);
  }
  await page.waitForLoadState("load");
  const url = await page.url();
  if (!url.startsWith(spec.expectedUrlPrefix)) {
    throw new Error(`Step "${spec.id}" ended on ${url}, expected a page under ${spec.expectedUrlPrefix}`);
  }
  return result.data.actions;
}

async function readText(page: Page, selector: string): Promise<string> {
  const locator = page.locator(selector);
  const count = await locator.count();
  if (count !== 1) {
    throw new Error(`Expected one element for "${selector}" on the author page, found ${count}.`);
  }
  return (await locator.textContent()).trim();
}

// Reads the author fields from fixed elements of the author page. No model call.
async function readAuthorDetails(page: Page): Promise<AuthorDetails> {
  const name = await readText(page, ".author-title");
  const birthDate = await readText(page, ".author-born-date");
  const birthplace = (await readText(page, ".author-born-location")).replace(/^in\s+/i, "");
  return { name, birthDate, birthplace };
}

async function main(): Promise<AuthorDetails> {
  const kept = await loadKeptSteps();
  const browser = await localBrowser.launch({ headless: true });
  const stagehand = await Stagehand.create({
    browser,
    model: { modelName: MODEL },
    selfHeal: false,
    cache: false,
  });

  try {
    const page = await activePage(stagehand);
    await page.goto(SITE, { waitUntil: "load" });

    const actionsById = new Map<string, Action[]>();
    const modelSteps: string[] = [];

    for (const spec of STEP_SPECS) {
      const keptActions = kept.get(spec.id);
      if (keptActions !== undefined) {
        const outcome = await replayStep(stagehand, page, spec, keptActions);
        if (outcome.ok) {
          actionsById.set(spec.id, keptActions);
          continue;
        }
        console.log(`Step "${spec.id}" no longer applies (${outcome.reason}). Sending only this step to the model.`);
      } else {
        console.log(`Step "${spec.id}" has no kept resolution. Resolving it with the model.`);
      }
      actionsById.set(spec.id, await resolveWithModel(stagehand, page, spec));
      modelSteps.push(spec.id);
    }

    if (modelSteps.length > 0) {
      await saveKeptSteps(actionsById);
      console.log(`Steps resolved by the model this run: ${modelSteps.join(", ")}.`);
    } else {
      console.log("All steps replayed from the kept file; no model call was made.");
    }

    // Plain string check, no model.
    const url = await page.url();
    if (!url.startsWith(AUTHOR_PREFIX)) {
      throw new Error(`The page reached is not an author page: ${url}`);
    }

    const details = await readAuthorDetails(page);
    console.log(JSON.stringify(details, null, 2));
    return details;
  } finally {
    await stagehand.close();
  }
}

try {
  await main();
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
}
