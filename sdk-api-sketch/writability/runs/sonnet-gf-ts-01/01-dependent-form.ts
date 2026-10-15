import { Collie, GoodfetchError } from "@goodfetch/collie";
import type { Page } from "@goodfetch/collie";
import { z } from "zod";

const URL_TO_OPEN = "https://quotes.toscrape.com/search.aspx";
const AUTHOR = "Albert Einstein";
const TAG = "inspirational";
const STEP_TIMEOUT = 15_000;

const QuoteSchema = z.object({
  text: z.string().min(1),
  author: z.string().min(1),
  tag: z.string().min(1),
});
type Quote = z.infer<typeof QuoteSchema>;

/** A failure that says which step of the task failed and what was seen instead. */
class StepError extends Error {
  readonly step: string;
  readonly observed: string;
  constructor(step: string, observed: string, cause?: unknown) {
    super(`Step "${step}" failed: ${observed}`, cause === undefined ? undefined : { cause });
    this.name = "StepError";
    this.step = step;
    this.observed = observed;
  }
}

function describeFailure(error: unknown): string {
  if (error instanceof GoodfetchError) {
    const parts = [`${error.code}: ${error.message}`];
    if (error.observed !== undefined) parts.push(`observed: ${error.observed}`);
    if (error.fix !== undefined) parts.push(`fix: ${error.fix}`);
    return parts.join("; ");
  }
  return error instanceof Error ? error.message : String(error);
}

async function step<T>(name: string, action: () => Promise<T>, observe?: () => Promise<string>): Promise<T> {
  try {
    return await action();
  } catch (error) {
    let seen = describeFailure(error);
    if (observe !== undefined) {
      try {
        seen += `; page showed: ${await observe()}`;
      } catch {
        // The observation is best effort.
      }
    }
    throw new StepError(name, seen, error);
  }
}

async function optionsOf(page: Page, selector: string): Promise<string> {
  const texts = await page.locator(`${selector} option`).allTextContents();
  return JSON.stringify(texts.map((t) => t.trim()));
}

async function searchQuotes(page: Page): Promise<Quote[]> {
  await step(
    "open the search page",
    async () => {
      await page.goto(URL_TO_OPEN, { waitUntil: "load", timeout: STEP_TIMEOUT });
      await page.locator("#author").waitFor({ state: "visible", timeout: STEP_TIMEOUT });
    },
    async () => `url ${page.url()}, title ${JSON.stringify(await page.title())}`,
  );

  await step(
    `choose the author "${AUTHOR}"`,
    async () => {
      const chosen = await page.locator("#author").selectOption({ label: AUTHOR }, { timeout: STEP_TIMEOUT });
      if (chosen.length !== 1) {
        throw new Error(`expected one selected author, got ${JSON.stringify(chosen)}`);
      }
    },
    async () => `authors offered ${await optionsOf(page, "#author")}`,
  );

  // The page reloads after the author is chosen and then offers that author's tags.
  await step(
    `wait for the tag list of "${AUTHOR}" to offer "${TAG}"`,
    async () => {
      await page.locator("#tag option", { hasText: TAG }).first().waitFor({ state: "attached", timeout: STEP_TIMEOUT });
    },
    async () => `tags offered ${await optionsOf(page, "#tag")}`,
  );

  await step(
    `choose the tag "${TAG}"`,
    async () => {
      const chosen = await page.locator("#tag").selectOption({ label: TAG }, { timeout: STEP_TIMEOUT });
      if (chosen.length !== 1) {
        throw new Error(`expected one selected tag, got ${JSON.stringify(chosen)}`);
      }
    },
    async () => `tags offered ${await optionsOf(page, "#tag")}`,
  );

  await step(
    "press Search",
    async () => {
      await page.getByRole("button", { name: "Search" }).click({ timeout: STEP_TIMEOUT });
      await page.waitForLoadState("networkidle", { timeout: STEP_TIMEOUT });
    },
    async () => `url ${page.url()}`,
  );

  // An empty result is a valid answer: give the results a short moment to appear, then count.
  let found = await page.locator(".quote").count();
  if (found === 0) {
    try {
      found = await page.locator(".quote").waitForCount({ min: 1 }, { timeout: 3_000 });
    } catch (error) {
      if (!(error instanceof GoodfetchError) || error.code !== "deadline_exceeded") {
        throw new StepError("read the results", describeFailure(error), error);
      }
      found = 0;
    }
  }

  return step(
    "read the quotes shown",
    async () => {
      const quotes: Quote[] = [];
      const blocks = await page.locator(".quote").all();
      if (blocks.length !== found && blocks.length === 0 && found > 0) {
        throw new Error(`counted ${found} quotes but could read none`);
      }
      for (const [index, block] of blocks.entries()) {
        const text = (await block.locator(".content").innerText({ timeout: STEP_TIMEOUT })).trim();
        const author = (await block.locator(".author").innerText({ timeout: STEP_TIMEOUT })).trim();
        const tags = (await block.locator(".tag").allInnerTexts()).map((t) => t.trim());
        if (!tags.includes(TAG)) {
          throw new Error(`quote ${index + 1} shows tags ${JSON.stringify(tags)}, not "${TAG}"`);
        }
        quotes.push(QuoteSchema.parse({ text, author, tag: TAG }));
      }
      return quotes;
    },
    async () => `${await page.locator(".quote").count()} quote blocks at ${page.url()}`,
  );
}

await using collie = await Collie.connect();
await using run = await collie.start();
await using page = await run.newPage();

const quotes = await searchQuotes(page);

if (quotes.length === 0) {
  console.log(`No quote matches author "${AUTHOR}" with tag "${TAG}": returning an empty list.`);
} else {
  console.log(`Found ${quotes.length} quote(s) by "${AUTHOR}" tagged "${TAG}".`);
}
console.log(JSON.stringify(quotes, null, 2));

await run.finish();
