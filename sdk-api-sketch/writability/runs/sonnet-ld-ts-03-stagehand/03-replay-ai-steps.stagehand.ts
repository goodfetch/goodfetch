import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { localBrowser, Stagehand, type Action, type Page } from "@browserbasehq/stagehand";
import { z } from "zod";

const START_URL = "https://quotes.toscrape.com/";
const AUTHOR_URL_PREFIX = "https://quotes.toscrape.com/author/";

// What the model resolved for each step lives next to this program.
const STEPS_FILE = join(dirname(fileURLToPath(import.meta.url)), "03-replay-ai-steps.steps.json");

const ActionSchema = z.object({
  selector: z.string(),
  description: z.string(),
  method: z.string().optional(),
  arguments: z.array(z.string()).optional(),
});
const StepFileSchema = z.object({
  version: z.literal(1),
  steps: z.record(z.string(), z.array(ActionSchema)),
});
type StepStore = z.infer<typeof StepFileSchema>["steps"];

type Step = {
  name: string;
  instruction: string;
  // Cheap, model-free check that the step really led where it should.
  reached: (url: string) => boolean;
};

const STEPS: Step[] = [
  {
    name: "open tag",
    instruction: 'Click the tag "love" in the list of tags',
    reached: (url) => /^https:\/\/quotes\.toscrape\.com\/tag\/love\/?$/.test(url),
  },
  {
    name: "second page",
    instruction: 'Click the "Next" button at the bottom of the quotes to go to the second page',
    reached: (url) => /^https:\/\/quotes\.toscrape\.com\/tag\/love\/page\/2\/?$/.test(url),
  },
  {
    name: "author about page",
    instruction: 'Click the "(about)" link next to the author of the first quote',
    reached: (url) => url.startsWith(AUTHOR_URL_PREFIX),
  },
];

function loadStore(): StepStore {
  if (!existsSync(STEPS_FILE)) return {};
  try {
    return StepFileSchema.parse(JSON.parse(readFileSync(STEPS_FILE, "utf8"))).steps;
  } catch {
    console.log(`Kept steps file ${STEPS_FILE} is unreadable: every step goes to the model.`);
    return {};
  }
}

function saveStore(steps: StepStore): void {
  writeFileSync(STEPS_FILE, JSON.stringify({ version: 1, steps }, null, 2) + "\n");
}

async function settle(page: Page): Promise<void> {
  await page.waitForLoadState("load", 15_000).catch(() => undefined);
}

const browser = await localBrowser.launch({ headless: true });
let stagehand: Stagehand | undefined;
try {
  const apiKey = process.env["OPENAI_API_KEY"];
  stagehand = await Stagehand.create({
    browser,
    // Kept steps are replayed as plain actions: a failing one must not call the model by itself,
    // the program decides which step goes back to the model.
    selfHeal: false,
    model: { modelName: "openai/gpt-5.4-mini", ...(apiKey ? { apiKey } : {}) },
  });

  const page = (await browser.context.pages())[0] ?? (await browser.context.newPage());
  await page.goto(START_URL, { waitUntil: "load" });

  const store = loadStore();
  let modelCalls = 0;

  for (const [index, step] of STEPS.entries()) {
    const label = `step ${index + 1} (${step.name}: "${step.instruction}")`;
    const kept = store[step.instruction];
    const urlBefore = await page.url();
    let done = false;

    if (kept && kept.length > 0) {
      try {
        for (const action of kept) {
          const result = await stagehand.act(action as Action, { page });
          if (!result.data.success) throw new Error(result.data.message);
          await settle(page);
        }
        done = step.reached(await page.url());
        if (!done) throw new Error(`the page reached is ${await page.url()}, not the expected one`);
        console.log(`${label}: replayed from the kept steps, no model call.`);
      } catch (error) {
        const reason = error instanceof Error ? error.message : String(error);
        console.log(`${label}: the kept step no longer applies (${reason}). Only this step goes back to the model.`);
        if ((await page.url()) !== urlBefore) {
          await page.goBack({ waitUntil: "load" });
          await settle(page);
        }
        delete store[step.instruction];
      }
    }

    if (!done) {
      modelCalls += 1;
      const result = await stagehand.act(step.instruction, { page });
      if (!result.data.success) throw new Error(`${label} failed with the model: ${result.data.message}`);
      await settle(page);
      const url = await page.url();
      if (!step.reached(url)) throw new Error(`${label} led to an unexpected page: ${url}`);
      store[step.instruction] = result.data.actions;
      saveStore(store);
      console.log(`${label}: resolved by the model and kept.`);
    }
  }

  // Confirm, without a model, that the page reached is an author page.
  const finalUrl = await page.url();
  if (!finalUrl.startsWith(AUTHOR_URL_PREFIX)) {
    throw new Error(`Expected an author page (${AUTHOR_URL_PREFIX}...), reached ${finalUrl}`);
  }

  // Read the author's details from the page itself, again without a model.
  const author = z
    .object({ name: z.string(), birthDate: z.string(), birthplace: z.string() })
    .parse(
      await page.evaluate(`(() => {
        const text = (selector) => (document.querySelector(selector)?.textContent ?? "").trim();
        return {
          name: text(".author-title"),
          birthDate: text(".author-born-date"),
          birthplace: text(".author-born-location").replace(/^in\\s+/i, ""),
        };
      })()`),
    );
  if (author.name === "" || author.birthDate === "" || author.birthplace === "") {
    throw new Error(`The author page ${finalUrl} did not show a name, a birth date and a birthplace`);
  }

  console.log(`Model calls in this run: ${modelCalls}`);
  console.log(JSON.stringify({ url: finalUrl, ...author }, null, 2));
} finally {
  await stagehand?.close().catch(() => undefined);
  await browser.close().catch(() => undefined);
}
