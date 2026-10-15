import { Collie } from "@goodfetch/collie";
import type { Locator, Run } from "@goodfetch/collie";

export interface Quote {
  readonly text: string;
  readonly author: string;
  readonly tag: string;
}

const SEARCH_URL = "https://quotes.toscrape.com/search.aspx";
const SEARCH_RESPONSE = /\/search\.aspx/;
const AUTHOR = "Albert Einstein";
const TAG = "inspirational";
const WAIT_MS = 30_000;

/** Runs one named step; a failure is rethrown with the step's name in front of what went wrong. */
async function step<T>(name: string, action: () => Promise<T>): Promise<T> {
  try {
    return await action();
  } catch (failure) {
    const detail = failure instanceof Error ? failure.message : String(failure);
    throw new Error(`Step "${name}" failed: ${detail}`);
  }
}

/** The visible text of a page, flattened and shortened so an error can show what was seen. */
function snippet(text: string): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > 200 ? `${flat.slice(0, 200)}...` : flat;
}

/** The labels of a select's options, without blank ones. */
async function optionLabels(select: Locator): Promise<string[]> {
  const labels = await select.locator("option").allTextContents();
  return labels.map((label) => label.trim()).filter((label) => label !== "");
}

async function readQuotes(run: Run): Promise<Quote[]> {
  await using page = await run.newPage();

  const author = page.locator("#author");
  const tag = page.locator("#tag");
  const search = page.getByRole("button", { name: "Search" });

  await step("open the search page", async () => {
    const response = await page.goto(SEARCH_URL, { timeout: WAIT_MS });
    if (response !== null && !response.ok()) {
      throw new Error(`the page answered ${response.status()} ${response.statusText()}`);
    }
  });

  await step("choose the author", async () => {
    const authors = await optionLabels(author);
    if (!authors.includes(AUTHOR)) {
      throw new Error(`no author "${AUTHOR}" in the author list; saw ${JSON.stringify(authors)}`);
    }
    // The author choice reloads the page: wait for that response and its load, not for the option.
    const reloaded = page.waitForResponse(SEARCH_RESPONSE, { timeout: WAIT_MS });
    await Promise.all([reloaded, author.selectOption({ label: AUTHOR })]);
    await page.waitForLoadState("load", { timeout: WAIT_MS });
  });

  await step("check the tag list after the author", async () => {
    const tags = await optionLabels(tag);
    if (!tags.includes(TAG)) {
      throw new Error(`the tag list offers ${JSON.stringify(tags)} after "${AUTHOR}", not "${TAG}"`);
    }
  });

  await step("choose the tag", async () => {
    await tag.selectOption({ label: TAG }, { timeout: WAIT_MS });
  });

  await step("press Search", async () => {
    const results = page.waitForResponse(SEARCH_RESPONSE, { timeout: WAIT_MS });
    await Promise.all([results, search.click({ timeout: WAIT_MS })]);
    await page.waitForLoadState("load", { timeout: WAIT_MS });
  });

  return step("read the results", async () => {
    const cards = page.locator("div.quote");
    const count = await cards.count();
    if (count === 0) {
      const body = await page.locator("body").innerText({ timeout: WAIT_MS });
      if (/no quotes found/i.test(body)) {
        console.log(`No quote matches the author "${AUTHOR}" with the tag "${TAG}".`);
        return [];
      }
      throw new Error(`no quote is shown and no "No quotes found" notice either; the page showed: ${snippet(body)}`);
    }

    const quotes: Quote[] = [];
    for (const card of await cards.all()) {
      const text = await card.locator("span.text").textContent({ timeout: WAIT_MS });
      const cardAuthor = await card.locator("small.author").textContent({ timeout: WAIT_MS });
      quotes.push({ text: text.trim(), author: cardAuthor.trim(), tag: TAG });
    }
    return quotes;
  });
}

export async function findEinsteinInspirationalQuotes(): Promise<Quote[]> {
  await using collie = await Collie.connect();
  await using run = await collie.start();
  const quotes = await readQuotes(run);
  await run.finish();
  return quotes;
}

const quotes = await findEinsteinInspirationalQuotes();
console.log(JSON.stringify(quotes, null, 2));
