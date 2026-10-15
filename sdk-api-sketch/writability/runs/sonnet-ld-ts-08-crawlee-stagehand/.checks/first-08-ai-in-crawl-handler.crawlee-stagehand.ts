// Crawl the first three categories of books.toscrape.com and extract each book's
// title, price, stock and star rating with a language model, asked once per page
// type. Later pages of the same layout are read through the selectors the model
// returned (no model call). When the crawl has spent BUDGET_USD the model is
// switched off, the crawl finishes without it and the books left unextracted are
// reported with the reason.

import { Dataset, StagehandCrawler } from '@crawlee/stagehand';
import type { StagehandCrawlingContext, StagehandPage } from '@crawlee/stagehand';
import type { Stagehand } from '@browserbasehq/stagehand';

// ---------------------------------------------------------------- configuration

const START_URL = 'https://books.toscrape.com/';
const CATEGORY_COUNT = 3;
const BUDGET_USD = Number(process.env.BUDGET_USD ?? '0.50');
const MODEL = process.env.STAGEHAND_MODEL ?? 'openai/gpt-4.1-mini';

// USD per million tokens. Defaults are the list prices of gpt-4.1-mini; override
// with PRICE_INPUT_PER_M / PRICE_CACHED_PER_M / PRICE_OUTPUT_PER_M for another model.
const PRICE_INPUT_PER_M = Number(process.env.PRICE_INPUT_PER_M ?? '0.40');
const PRICE_CACHED_PER_M = Number(process.env.PRICE_CACHED_PER_M ?? '0.10');
const PRICE_OUTPUT_PER_M = Number(process.env.PRICE_OUTPUT_PER_M ?? '1.60');

const FIELDS = ['title', 'price', 'stock', 'rating'] as const;
type Field = (typeof FIELDS)[number];

const RATING_WORDS: Record<string, number> = { one: 1, two: 2, three: 3, four: 4, five: 5 };

// ---------------------------------------------------------------- types

interface Book {
    url: string;
    category: string;
    title: string;
    price: number;
    currency: string;
    inStock: number;
    rating: number;
    source: 'model' | 'learned';
}

interface Unextracted {
    url: string;
    category: string;
    reason: string;
}

type Selectors = Record<Field, string>;

interface Usage {
    promptTokens: number;
    completionTokens: number;
    reasoningTokens: number;
    cachedInputTokens: number;
}

type Reading = { ok: true; book: Omit<Book, 'url' | 'category' | 'source'> } | { ok: false; reason: string };

// ---------------------------------------------------------------- state

const books: Book[] = [];
const unextracted: Unextracted[] = [];

const usage: Usage = { promptTokens: 0, completionTokens: 0, reasoningTokens: 0, cachedInputTokens: 0 };
let modelCalls = 0;
let modelStoppedReason: string | null = null;

/** What the model learned for the book page type. `version` changes on every relearn. */
let learned: { selectors: Selectors; version: number } | null = null;
let relearns = 0;

function costUsd(u: Usage): number {
    const uncachedInput = Math.max(0, u.promptTokens - u.cachedInputTokens);
    // Reasoning tokens are billed as output; counting them separately may slightly
    // overstate the cost when a provider already folds them into completion tokens.
    const output = u.completionTokens + u.reasoningTokens;
    return (
        (uncachedInput * PRICE_INPUT_PER_M + u.cachedInputTokens * PRICE_CACHED_PER_M + output * PRICE_OUTPUT_PER_M) /
        1_000_000
    );
}

/** One model call at a time, so the spend is known exactly before the next call decides. */
let modelLock: Promise<unknown> = Promise.resolve();
function exclusive<T>(fn: () => Promise<T>): Promise<T> {
    const run = modelLock.then(fn, fn);
    modelLock = run.catch(() => undefined);
    return run;
}

async function readMetrics(stagehand: Stagehand): Promise<Usage> {
    const m = await stagehand.metrics;
    return {
        promptTokens: m.totalPromptTokens,
        completionTokens: m.totalCompletionTokens,
        reasoningTokens: m.totalReasoningTokens,
        cachedInputTokens: m.totalCachedInputTokens,
    };
}

/** Runs one model call and adds the tokens it used to the crawl's ledger, even when it fails. */
async function metered<T>(stagehand: Stagehand, call: () => Promise<T>): Promise<T> {
    const before = await readMetrics(stagehand);
    modelCalls++;
    try {
        return await call();
    } finally {
        const after = await readMetrics(stagehand);
        usage.promptTokens += after.promptTokens - before.promptTokens;
        usage.completionTokens += after.completionTokens - before.completionTokens;
        usage.reasoningTokens += after.reasoningTokens - before.reasoningTokens;
        usage.cachedInputTokens += after.cachedInputTokens - before.cachedInputTokens;
        if (modelStoppedReason === null && costUsd(usage) >= BUDGET_USD) {
            modelStoppedReason = `model budget of ${BUDGET_USD.toFixed(2)} USD spent`;
        }
    }
}

// ---------------------------------------------------------------- reading a page

function parseReading(raw: Record<Field, string | null>): Reading {
    const title = raw.title?.trim() ?? '';
    if (title.length === 0) return { ok: false, reason: 'title selector matched no text' };

    const priceMatch = /([^\d\s.,]+)\s*(\d+(?:[.,]\d+)?)/.exec(raw.price ?? '');
    if (!priceMatch?.[1] || !priceMatch[2]) return { ok: false, reason: `price text not recognised: "${raw.price ?? ''}"` };

    const stockMatch = /(\d+)\s+available/i.exec(raw.stock ?? '');
    if (!stockMatch?.[1]) return { ok: false, reason: `stock text not recognised: "${(raw.stock ?? '').trim()}"` };

    const ratingMatch = /\b(one|two|three|four|five)\b/i.exec(raw.rating ?? '');
    const rating = ratingMatch?.[1] ? RATING_WORDS[ratingMatch[1].toLowerCase()] : undefined;
    if (rating === undefined) return { ok: false, reason: `star rating not recognised: "${(raw.rating ?? '').trim()}"` };

    return {
        ok: true,
        book: {
            title,
            price: Number(priceMatch[2].replace(',', '.')),
            currency: priceMatch[1],
            inStock: Number(stockMatch[1]),
            rating,
        },
    };
}

/** Reads the four fields through selectors, with no model call. */
async function readWithSelectors(page: StagehandPage, selectors: Selectors): Promise<Reading> {
    const raw: Record<Field, string | null> = { title: null, price: null, stock: null, rating: null };
    for (const field of FIELDS) {
        const el = page.locator(selectors[field]).first();
        try {
            if ((await el.count()) === 0) return { ok: false, reason: `${field} selector matched nothing` };
            if (field === 'rating') {
                // The rating is the word in the class of the star element (or of its parent).
                const own = (await el.getAttribute('class', { timeout: 2000 })) ?? '';
                const parent = (await el.locator('xpath=..').getAttribute('class', { timeout: 2000 })) ?? '';
                raw.rating = `${own} ${parent}`;
            } else {
                raw.title = field === 'title' ? await el.textContent({ timeout: 2000 }) : raw.title;
                raw.price = field === 'price' ? await el.textContent({ timeout: 2000 }) : raw.price;
                raw.stock = field === 'stock' ? await el.textContent({ timeout: 2000 }) : raw.stock;
            }
        } catch (error) {
            return { ok: false, reason: `${field} selector failed: ${error instanceof Error ? error.message : String(error)}` };
        }
    }
    return parseReading(raw);
}

// ---------------------------------------------------------------- the one model call

const OBSERVE_INSTRUCTION =
    'This is a book detail page. Return exactly four elements, in this order, and start each description ' +
    'with the field name followed by a colon: ' +
    '"title:" the main heading with the book title; ' +
    '"price:" the element showing the book price; ' +
    '"stock:" the element showing availability and the number of copies in stock; ' +
    '"rating:" the star rating element of the book (its star rating is carried by its class).';

async function learnFromModel(page: StagehandPage, stagehand: Stagehand): Promise<{ selectors: Selectors } | { reason: string }> {
    let actions;
    try {
        actions = await metered(stagehand, () => page.observe(OBSERVE_INSTRUCTION));
    } catch (error) {
        return { reason: `model call failed: ${error instanceof Error ? error.message : String(error)}` };
    }

    const selectors: Partial<Selectors> = {};
    for (const field of FIELDS) {
        const byName = actions.find((a) => a.description.toLowerCase().startsWith(field));
        if (byName) selectors[field] = byName.selector;
    }
    // Fall back to the requested order when the descriptions were not labelled.
    if (FIELDS.some((f) => selectors[f] === undefined) && actions.length === FIELDS.length) {
        FIELDS.forEach((f, i) => {
            const a = actions[i];
            if (selectors[f] === undefined && a) selectors[f] = a.selector;
        });
    }
    const [title, price, stock, rating] = [selectors.title, selectors.price, selectors.stock, selectors.rating];
    if (title === undefined || price === undefined || stock === undefined || rating === undefined) {
        return { reason: `model returned ${actions.length} elements, not one for each of title, price, stock and rating` };
    }
    const complete: Selectors = { title, price, stock, rating };

    // The model's selectors must reproduce valid values on the page they were learned from.
    const check = await readWithSelectors(page, complete);
    if (!check.ok) return { reason: `selectors from the model did not read the page: ${check.reason}` };
    return { selectors: complete };
}

// ---------------------------------------------------------------- book handler

async function handleBook(ctx: StagehandCrawlingContext): Promise<void> {
    const { page, request, stagehand, log } = ctx;
    const category = typeof request.userData.category === 'string' ? request.userData.category : '';
    const url = request.loadedUrl ?? request.url;

    const record = async (reading: { book: Reading extends infer R ? (R extends { ok: true } ? R['book'] : never) : never }, source: Book['source']): Promise<void> => {
        const book: Book = { url, category, source, ...reading.book };
        books.push(book);
        await Dataset.pushData(book);
    };
    const skip = (reason: string): void => {
        unextracted.push({ url, category, reason });
        log.warning(`Book left unextracted: ${url} (${reason})`);
    };

    // 1. What was learned so far, with no model call.
    const seen = learned;
    let failure = 'no layout learned yet';
    if (seen !== null) {
        const reading = await readWithSelectors(page, seen.selectors);
        if (reading.ok) return record(reading, 'learned');
        failure = `page no longer matches what was learned: ${reading.reason}`;
    }

    // 2. First book of the page type, or a page that no longer matches: ask the model,
    //    one call at a time and only while the budget lasts.
    type Outcome = { ok: true; book: Book extends infer B ? Omit<Extract<B, object>, 'url' | 'category' | 'source'> : never; source: Book['source'] } | { ok: false; reason: string };
    const outcome = await exclusive(async (): Promise<Outcome> => {
        // Another handler may have learned (or relearned) while this one waited.
        if (learned !== null && learned.version !== seen?.version) {
            const reading = await readWithSelectors(page, learned.selectors);
            if (reading.ok) return { ok: true, book: reading.book, source: 'learned' };
        }
        if (modelStoppedReason !== null) {
            return { ok: false, reason: `${failure}; model stopped (${modelStoppedReason})` };
        }
        const result = await learnFromModel(page, stagehand);
        if ('reason' in result) return { ok: false, reason: `${failure}; ${result.reason}` };
        const reading = await readWithSelectors(page, result.selectors);
        if (!reading.ok) return { ok: false, reason: `${failure}; ${reading.reason}` };
        if (learned !== null) relearns++;
        learned = { selectors: result.selectors, version: (learned?.version ?? 0) + 1 };
        log.info(`Learned the book layout from the model (version ${learned.version}): ${JSON.stringify(result.selectors)}`);
        return { ok: true, book: reading.book, source: 'model' };
    });

    if (outcome.ok) {
        await record(outcome, outcome.source);
    } else {
        skip(outcome.reason);
    }
}

// ---------------------------------------------------------------- crawler

const crawler = new StagehandCrawler({
    stagehandOptions: { env: 'LOCAL', model: MODEL, verbose: 0 },
    maxConcurrency: 4,
    maxRequestRetries: 2,
    async requestHandler(ctx) {
        const { request, page, enqueueLinks, log } = ctx;
        const label = request.label ?? 'START';

        if (label === 'START') {
            // The first three categories of the side list.
            const links = page.locator('.side_categories ul.nav-list > li > ul > li > a');
            const urls: string[] = [];
            for (let i = 0; i < CATEGORY_COUNT; i++) {
                const href = await links.nth(i).getAttribute('href');
                if (href) urls.push(new URL(href, request.loadedUrl ?? request.url).href);
            }
            log.info(`Categories: ${urls.join(', ')}`);
            await enqueueLinks({ urls, label: 'CATEGORY' });
            return;
        }

        if (label === 'CATEGORY') {
            const heading = (await page.locator('h1').first().textContent()) ?? '';
            const category =
                typeof request.userData.category === 'string' && request.userData.category !== ''
                    ? request.userData.category
                    : heading.trim();
            await enqueueLinks({ selector: 'article.product_pod h3 a', label: 'BOOK', userData: { category } });
            await enqueueLinks({ selector: 'li.next a', label: 'CATEGORY', userData: { category } });
            return;
        }

        await handleBook(ctx);
    },
    async failedRequestHandler({ request, log }) {
        if (request.label === 'BOOK') {
            const category = typeof request.userData.category === 'string' ? request.userData.category : '';
            const reason = `request failed after retries: ${request.errorMessages.at(-1) ?? 'unknown error'}`;
            unextracted.push({ url: request.url, category, reason });
            log.error(`Book left unextracted: ${request.url} (${reason})`);
        }
    },
});

await crawler.run([{ url: START_URL, label: 'START' }]);

// ---------------------------------------------------------------- report

const spent = costUsd(usage);
const byReason = new Map<string, number>();
for (const u of unextracted) byReason.set(u.reason, (byReason.get(u.reason) ?? 0) + 1);

console.log(
    JSON.stringify(
        {
            model: MODEL,
            budgetUsd: BUDGET_USD,
            modelStopped: modelStoppedReason,
            booksExtracted: books.length,
            extractedByModel: books.filter((b) => b.source === 'model').length,
            extractedFromLearned: books.filter((b) => b.source === 'learned').length,
            booksUnextracted: unextracted.length,
            unextractedByReason: Object.fromEntries(byReason),
            unextracted,
            modelCalls,
            relearns,
            tokens: {
                prompt: usage.promptTokens,
                cachedInput: usage.cachedInputTokens,
                completion: usage.completionTokens,
                reasoning: usage.reasoningTokens,
                total: usage.promptTokens + usage.completionTokens + usage.reasoningTokens,
            },
            costUsd: Number(spent.toFixed(6)),
            pricePerMillionTokensUsd: { input: PRICE_INPUT_PER_M, cachedInput: PRICE_CACHED_PER_M, output: PRICE_OUTPUT_PER_M },
        },
        null,
        2,
    ),
);
