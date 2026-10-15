import { chromium, errors } from 'playwright';
import type { Browser, Page } from 'playwright';

const URL = 'https://quotes.toscrape.com/search.aspx';
const AUTHOR = 'Albert Einstein';
const TAG = 'inspirational';
const STEP_TIMEOUT_MS = 15_000;

interface Quote {
  text: string;
  author: string;
  tag: string;
}

class StepError extends Error {
  constructor(step: string, seen: string, options?: ErrorOptions) {
    super(`Step "${step}" failed: ${seen}`, options);
    this.name = 'StepError';
  }
}

async function optionsOf(page: Page, selector: string): Promise<string[]> {
  const texts = await page.locator(`${selector} option`).allTextContents();
  return texts.map((t) => t.trim());
}

async function searchQuotes(browser: Browser): Promise<Quote[]> {
  const context = await browser.newContext();
  try {
    const page = await context.newPage();
    page.setDefaultTimeout(STEP_TIMEOUT_MS);

    // Step 1: open the page.
    try {
      const response = await page.goto(URL, { waitUntil: 'load' });
      if (response === null || !response.ok()) {
        throw new StepError(
          'open the search page',
          `response status ${response === null ? 'none' : String(response.status())} for ${URL}`,
        );
      }
    } catch (error) {
      if (error instanceof StepError) throw error;
      throw new StepError('open the search page', `navigation to ${URL} failed: ${String(error)}`, {
        cause: error,
      });
    }

    const authorSelect = page.locator('#author');
    const tagSelect = page.locator('#tag');

    // Step 2: choose the author; the page reloads and then lists that author's tags.
    try {
      await authorSelect.waitFor({ state: 'attached' });
    } catch (error) {
      throw new StepError(
        'find the author list',
        `no #author select on the page (title "${await page.title()}")`,
        { cause: error },
      );
    }
    const authors = await optionsOf(page, '#author');
    if (!authors.includes(AUTHOR)) {
      throw new StepError(
        'choose the author',
        `author "${AUTHOR}" is not offered; the list holds: ${JSON.stringify(authors)}`,
      );
    }
    await authorSelect.selectOption({ label: AUTHOR });

    // Step 3: wait for the tag list to offer the author's tags.
    const tagOption = tagSelect.locator('option', { hasText: TAG });
    try {
      await tagOption.first().waitFor({ state: 'attached' });
    } catch (error) {
      if (!(error instanceof errors.TimeoutError)) throw error;
      const tags = await optionsOf(page, '#tag');
      throw new StepError(
        'wait for the tags of the chosen author',
        `after choosing "${AUTHOR}" the tag list did not offer "${TAG}"; it holds: ${JSON.stringify(tags)}`,
        { cause: error },
      );
    }
    const tags = await optionsOf(page, '#tag');
    if (!tags.includes(TAG)) {
      throw new StepError(
        'choose the tag',
        `tag "${TAG}" is not offered; the list holds: ${JSON.stringify(tags)}`,
      );
    }
    await tagSelect.selectOption({ label: TAG });

    // Step 4: press Search.
    const searchButton = page.getByRole('button', { name: /search/i });
    try {
      await searchButton.first().waitFor({ state: 'visible' });
    } catch (error) {
      throw new StepError('press Search', 'no visible Search button on the page', { cause: error });
    }
    await searchButton.first().click();

    // Step 5: read the quotes shown.
    const quotes = page.locator('.quote');
    try {
      await quotes.first().waitFor({ state: 'visible' });
    } catch (error) {
      if (!(error instanceof errors.TimeoutError)) throw error;
      console.log(`No quote matches author "${AUTHOR}" with tag "${TAG}": the result list is empty.`);
      return [];
    }

    const count = await quotes.count();
    const result: Quote[] = [];
    for (let i = 0; i < count; i += 1) {
      const quote = quotes.nth(i);
      const text = (await quote.locator('.content').first().textContent())?.trim();
      const author = (await quote.locator('.author').first().textContent())?.trim();
      if (text === undefined || text === '' || author === undefined || author === '') {
        throw new StepError(
          'read the quotes',
          `quote ${String(i + 1)} of ${String(count)} lacks a text or an author: ${JSON.stringify(
            (await quote.textContent())?.trim() ?? '',
          )}`,
        );
      }
      result.push({ text, author, tag: TAG });
    }
    return result;
  } finally {
    await context.close();
  }
}

const browser = await chromium.launch();
try {
  const quotes = await searchQuotes(browser);
  console.log(JSON.stringify(quotes, null, 2));
} finally {
  await browser.close();
}
