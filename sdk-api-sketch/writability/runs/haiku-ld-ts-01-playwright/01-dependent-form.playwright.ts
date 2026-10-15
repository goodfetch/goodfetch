import { chromium, type Browser, type Locator, type Page } from 'playwright';

export interface Quote {
  text: string;
  author: string;
  tag: string;
}

const SEARCH_URL = 'https://quotes.toscrape.com/search.aspx';
const AUTHOR = 'Albert Einstein';
const TAG = 'inspirational';
const TIMEOUT_MS = 15_000;

class StepError extends Error {
  readonly step: string;
  readonly seen: string;

  constructor(step: string, seen: string) {
    super(`Step "${step}" failed: ${seen}`);
    this.name = 'StepError';
    this.step = step;
    this.seen = seen;
  }
}

function normalise(value: string): string {
  return value.replace(/\s+/g, ' ').trim();
}

function firstLine(message: string): string {
  const [first = message] = message.split('\n');
  return first.trim();
}

// Wraps any non-step failure (timeouts, missing elements) with the step name and the page URL seen at failure.
async function runStep<T>(step: string, page: Page, action: () => Promise<T>): Promise<T> {
  try {
    return await action();
  } catch (error) {
    if (error instanceof StepError) {
      throw error;
    }
    const reason = error instanceof Error ? firstLine(error.message) : String(error);
    throw new StepError(step, `${reason}; page URL was ${page.url()}`);
  }
}

async function optionLabels(select: Locator): Promise<string[]> {
  const labels = await select.locator('option').allTextContents();
  return labels.map(normalise).filter((label) => label !== '');
}

async function chooseAuthor(page: Page): Promise<void> {
  const authorSelect = page.locator('select[name="author"]');
  const labels = await optionLabels(authorSelect);
  if (!labels.includes(AUTHOR)) {
    throw new StepError(
      'choose author',
      `no option "${AUTHOR}" in the author list; options seen: ${JSON.stringify(labels)}`,
    );
  }
  // Choosing an author makes the page reload (postback); wait for the new document to load.
  await Promise.all([
    page.waitForEvent('load', { timeout: TIMEOUT_MS }),
    authorSelect.selectOption({ label: AUTHOR }),
  ]);
}

async function chooseTag(page: Page): Promise<void> {
  const tagSelect = page.locator('select[name="tag"]');
  const labels = await optionLabels(tagSelect);
  if (!labels.includes(TAG)) {
    throw new StepError(
      'choose tag',
      `no option "${TAG}" in the tag list after choosing "${AUTHOR}"; options seen: ${JSON.stringify(labels)}`,
    );
  }
  await tagSelect.selectOption({ label: TAG });
}

async function pressSearch(page: Page): Promise<void> {
  await Promise.all([
    page.waitForEvent('load', { timeout: TIMEOUT_MS }),
    page.getByRole('button', { name: 'Search', exact: true }).click(),
  ]);
}

async function readResults(page: Page): Promise<Quote[]> {
  const blocks = page.locator('div.quote');
  const blockCount = await blocks.count();
  if (blockCount === 0) {
    const bodyText = normalise(await page.locator('body').innerText());
    if (/no quotes found/i.test(bodyText)) {
      return [];
    }
    throw new StepError(
      'read results',
      `no quote blocks and no "No quotes found" message; page text starts with: ${bodyText.slice(0, 300)}`,
    );
  }

  const texts = (await blocks.locator('.text').allTextContents()).map(normalise);
  const authors = (await blocks.locator('.author').allTextContents()).map(normalise);
  if (texts.length !== blockCount || authors.length !== blockCount) {
    throw new StepError(
      'read results',
      `${blockCount} quote blocks but ${texts.length} quote texts and ${authors.length} authors`,
    );
  }

  return texts.map((text, index): Quote => {
    const author = authors[index];
    if (author !== AUTHOR) {
      throw new StepError(
        'read results',
        `quote ${index + 1} is by "${author}", expected "${AUTHOR}" after filtering`,
      );
    }
    // Every result matches the searched tag, so each quote is reported with that tag.
    return { text, author, tag: TAG };
  });
}

export async function findQuotes(): Promise<Quote[]> {
  const browser: Browser = await chromium.launch();
  try {
    const context = await browser.newContext();
    try {
      const page = await context.newPage();
      page.setDefaultTimeout(TIMEOUT_MS);
      await runStep('open page', page, async () => {
        await page.goto(SEARCH_URL, { waitUntil: 'load' });
      });
      await runStep('choose author', page, () => chooseAuthor(page));
      await runStep('choose tag', page, () => chooseTag(page));
      await runStep('press Search', page, () => pressSearch(page));
      return await runStep('read results', page, () => readResults(page));
    } finally {
      await context.close();
    }
  } finally {
    await browser.close();
  }
}

try {
  const quotes = await findQuotes();
  if (quotes.length === 0) {
    console.log(`No quotes match author "${AUTHOR}" with tag "${TAG}".`);
  }
  console.log(JSON.stringify(quotes, null, 2));
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
}
