// Snippet for Collie's AI verbs and Plans (#473, ADR 0091), on golden scenarios 02, 03, 09 and 11.
import { z } from "zod";
import {
  ExtractionIncompleteError,
  Goodfetch,
  PlanDriftError,
  expect,
  type Logger,
  type Page,
  type PlanReport,
  type Run,
} from "goodfetch";

declare const log: Logger; // the program's own logger

// Provisional, #478's: how a program finishes a Collie Run and reads its report.
declare function finish(run: Run): Promise<{ readonly verdict: string; readonly plan: PlanReport }>;

const Author = z.object({
  name: z.string(),
  birthDate: z.string(),
  birthplace: z.string(),
});

// Golden scenario 03, "Repeat AI steps with no model call". The first run compiles each step with the
// model; a later run replays them from quotes.plan.json with no model call, and a step that drifted
// goes back to the model once, alone. In CI, `settings: { plan: { frozen: true } }` forbids any model call.
export async function authorOfFirstLoveQuote(): Promise<z.infer<typeof Author>> {
  await using gf = await Goodfetch.connect({ logger: log });
  await using run = await gf.collie.start({ plan: "quotes.plan.json" });
  const page = await run.newPage();
  await page.goto("https://quotes.toscrape.com/");

  for (const instruction of [
    "open the tag 'love' from the tag list",
    "go to the second page",
    "open the about page of the first quote's author",
  ]) {
    const { step } = await page.act(instruction);
    if (step.source === "repair") log.warn(`step repaired: ${step.instruction}`, { drift: step.drift });
  }

  // A Success check, without a model: the Run is `verified` only if it holds, and only a verified
  // Run promotes a new Plan version (ADR 0009).
  await expect(page).toHaveURL("https://quotes.toscrape.com/author/**");
  const { output: author } = await page.extract("the author's name, birth date and birthplace", Author);

  const report = await finish(run);
  log.info(`plan ${report.plan.status}`, { changed: report.plan.changedSteps });
  return author;
}

const Book = z.object({
  title: z.string(),
  price: z.string().regex(/^\d+\.\d{2}$/), // pounds, exact
  inStock: z.number().int(),
  rating: z.number().int().min(1).max(5),
  description: z.string(),
});

// Golden scenario 02, "Extract a typed object with a chosen model": the model chosen for this call
// alone (#475 types it), and its usage next to the result. A field not on the page throws, naming it.
export async function typedBook(page: Page): Promise<z.infer<typeof Book> | undefined> {
  await page.goto("https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html");
  try {
    const { output, usage } = await page.extract("the book on this page", Book, { model: "ollama/qwen3:8b" });
    log.info("extracted", { tokens: usage.inputTokens + usage.outputTokens });
    return output;
  } catch (error) {
    if (error instanceof ExtractionIncompleteError) {
      log.warn("fields not on the page", { missing: error.details.missing, found: error.details.found });
      return undefined;
    }
    throw error;
  }
}

// Golden scenario 09, "Add an AI step to an existing Playwright program": the borrowed page gets the
// verbs, and ending the borrow never closes or navigates the program's own page (ADR 0078).
export async function travelBooks(run: Run, playwrightPage: { createCDPSession(): Promise<unknown> }): Promise<unknown> {
  await using page = await run.borrow(playwrightPage);
  const Listing = z.array(z.object({ title: z.string(), price: z.string() }));
  return (await page.locator("ol.row").extract("every book listed: title and price", Listing)).output;
}

// `observe` finds without acting; the program acts on a candidate through its ordinary locator.
// Values typed into the page travel as variables: the model and the Plan see only `%query%`.
export async function search(page: Page, query: string): Promise<void> {
  const { output: boxes } = await page.observe("the search box in the header");
  const [box] = boxes;
  if (box === undefined) return; // none is an answer
  await box.locator.fill(query);
  await page.act("type %query% into the search box and submit it", { variables: { query } });
}

// A program's own agent loop reads the observation and acts on references (Playwright 1.59+'s names).
declare function askModel(observation: string): Promise<{ readonly ref: string; readonly done: boolean }>;
export async function ownAgentLoop(page: Page): Promise<void> {
  for (let turn = 0; turn < 10; turn++) {
    const observation = await page.ariaSnapshot({ mode: "ai" }); // bounded by observation.max_size, page text marked untrusted
    const next = await askModel(observation);
    if (next.done) break;
    await page.getByRef(next.ref).click(); // references last as long as their element
  }
}

// A drift that cannot be repaired names the step and why.
export async function frozenReplay(page: Page): Promise<void> {
  try {
    await page.act("open the tag 'love' from the tag list");
  } catch (error) {
    if (error instanceof PlanDriftError) log.error(`${error.details.instruction}: ${error.details.whyNoRepair}`);
    throw error;
  }
}
