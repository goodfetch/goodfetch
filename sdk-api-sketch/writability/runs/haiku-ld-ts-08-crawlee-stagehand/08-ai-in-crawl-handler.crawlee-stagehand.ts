import { StagehandCrawler, type StagehandCrawlingContext } from '@crawlee/stagehand';
import { z } from 'zod';

const START_URL = 'https://books.toscrape.com/';
const CATEGORY_COUNT = 3;
const MODEL = 'openai/gpt-4.1-mini';
const BUDGET_USD = 0.5;
// USD per million tokens for MODEL, used to turn token counts into cost. Check against the provider's price list.
const PRICE_PER_MILLION_TOKENS = { input: 0.4, cachedInput: 0.1, output: 1.6 } as const;
const EXTRACT_INSTRUCTION = [
    'Extract the book on this page:',
    'its title;',
    'its price in pounds as a number, without the currency symbol;',
    'its number of copies in stock, from the "In stock (N available)" text;',
    'its star rating from 1 to 5, from the star-rating icons.',
].join(' ');
const RATING_WORDS = ['One', 'Two', 'Three', 'Four', 'Five'];

const BookSchema = z.object({
    title: z.string().min(1),
    price: z.number().positive(),
    inStock: z.number().int().nonnegative(),
    rating: z.number().int().min(1).max(5),
});
type BookFields = z.infer<typeof BookSchema>;

type BookPage = StagehandCrawlingContext['page'];
type StagehandInstance = StagehandCrawlingContext['stagehand'];

const FIELDS = ['title', 'price', 'inStock', 'rating'] as const;
type Field = (typeof FIELDS)[number];
type FieldValue = string | number;

/** CSS selector per book field, learned from a page the model extracted. */
type Recipe = Record<Field, string>;

type Method = 'layout' | 'model';
interface BookRecord extends BookFields {
    url: string;
    category: string;
    method: Method;
}
interface UnextractedBook {
    url: string;
    category: string;
    reason: string;
}
interface Usage {
    promptTokens: number;
    cachedInputTokens: number;
    completionTokens: number;
    reasoningTokens: number;
}
interface Ledger {
    usage: Usage;
    costUsd: number;
    modelCalls: number;
}

const recipes: Recipe[] = [];
const listed = new Map<string, string>(); // book URL -> category
const extracted = new Map<string, BookRecord>();
const unextracted = new Map<string, UnextractedBook>();
const ledger: Ledger = {
    usage: { promptTokens: 0, cachedInputTokens: 0, completionTokens: 0, reasoningTokens: 0 },
    costUsd: 0,
    modelCalls: 0,
};

const usd = (amount: number): string => `$${amount.toFixed(4)}`;
const errorText = (error: unknown): string => (error instanceof Error ? error.message : String(error));
const normalize = (text: string): string => text.replace(/\s+/g, ' ').trim();

function usageOf(stagehand: StagehandInstance): Usage {
    const metrics = stagehand.stagehandMetrics;
    return {
        promptTokens: metrics.totalPromptTokens,
        cachedInputTokens: metrics.totalCachedInputTokens,
        completionTokens: metrics.totalCompletionTokens,
        reasoningTokens: metrics.totalReasoningTokens,
    };
}

function diffUsage(after: Usage, before: Usage): Usage {
    return {
        promptTokens: after.promptTokens - before.promptTokens,
        cachedInputTokens: after.cachedInputTokens - before.cachedInputTokens,
        completionTokens: after.completionTokens - before.completionTokens,
        reasoningTokens: after.reasoningTokens - before.reasoningTokens,
    };
}

function sumUsage(a: Usage, b: Usage): Usage {
    return {
        promptTokens: a.promptTokens + b.promptTokens,
        cachedInputTokens: a.cachedInputTokens + b.cachedInputTokens,
        completionTokens: a.completionTokens + b.completionTokens,
        reasoningTokens: a.reasoningTokens + b.reasoningTokens,
    };
}

function costOf(usage: Usage): number {
    const uncachedInput = Math.max(0, usage.promptTokens - usage.cachedInputTokens);
    const output = usage.completionTokens + usage.reasoningTokens;
    return (
        (uncachedInput * PRICE_PER_MILLION_TOKENS.input +
            usage.cachedInputTokens * PRICE_PER_MILLION_TOKENS.cachedInput +
            output * PRICE_PER_MILLION_TOKENS.output) /
        1_000_000
    );
}

/** Reads one field from element text and classes; undefined when the element does not hold that field. */
function readField(field: Field, text: string, classes: readonly string[]): FieldValue | undefined {
    switch (field) {
        case 'title':
            return text.length > 0 ? text : undefined;
        case 'price': {
            const match = /£\s*(\d+(?:\.\d+)?)/.exec(text);
            return match ? Number(match[1]) : undefined;
        }
        case 'inStock': {
            const match = /In stock \((\d+) available\)/.exec(text);
            return match ? Number(match[1]) : undefined;
        }
        case 'rating': {
            if (!classes.includes('star-rating')) return undefined;
            const index = RATING_WORDS.findIndex((word) => classes.includes(word));
            return index === -1 ? undefined : index + 1;
        }
    }
}

async function readAt(page: BookPage, selector: string, field: Field): Promise<FieldValue | undefined> {
    const locator = page.locator(selector);
    if ((await locator.count()) !== 1) return undefined;
    const text = normalize((await locator.textContent()) ?? '');
    const classes = ((await locator.getAttribute('class')) ?? '').split(/\s+/).filter((name) => name.length > 0);
    return readField(field, text, classes);
}

/** Applies a learned recipe; returns undefined unless every field reads and the schema accepts the result. */
async function readWithRecipe(page: BookPage, recipe: Recipe): Promise<BookFields | undefined> {
    const values: Partial<Record<Field, FieldValue>> = {};
    for (const field of FIELDS) {
        const value = await readAt(page, recipe[field], field);
        if (value === undefined) return undefined;
        values[field] = value;
    }
    const parsed = BookSchema.safeParse(values);
    return parsed.success ? parsed.data : undefined;
}

interface Candidate {
    path: string;
    text: string;
    classes: string[];
}

/** Every element of the page with a CSS path that identifies it uniquely from the document root. */
async function collectCandidates(page: BookPage): Promise<Candidate[]> {
    return page.evaluate(() => {
        const pathOf = (element: Element): string => {
            const steps: string[] = [];
            for (let node: Element | null = element; node !== null && node.tagName !== 'HTML'; node = node.parentElement) {
                const tag = node.tagName;
                const parent = node.parentElement;
                const position =
                    parent === null ? 1 : Array.from(parent.children).filter((child) => child.tagName === tag).indexOf(node) + 1;
                const classes = Array.from(node.classList, (name) => `.${CSS.escape(name)}`).join('');
                steps.unshift(`${tag.toLowerCase()}${classes}:nth-of-type(${position})`);
            }
            return steps.join(' > ');
        };
        return Array.from(document.querySelectorAll('body *'), (element) => ({
            path: pathOf(element),
            text: element.textContent ?? '',
            classes: Array.from(element.classList),
        })).filter((candidate) => candidate.text.length <= 400);
    });
}

/**
 * Learns a recipe from a page the model just extracted: for each field, the first (innermost) element whose
 * value, read the same way the recipe reads it, equals the model's answer and whose selector finds exactly that element.
 */
async function learnRecipe(page: BookPage, book: BookFields): Promise<Recipe | undefined> {
    const candidates = (await collectCandidates(page))
        .map((candidate) => ({ ...candidate, text: normalize(candidate.text) }))
        .sort((a, b) => a.text.length - b.text.length);
    const expected: Record<Field, FieldValue> = {
        title: book.title,
        price: book.price,
        inStock: book.inStock,
        rating: book.rating,
    };
    const found: Partial<Recipe> = {};
    for (const field of FIELDS) {
        for (const candidate of candidates) {
            if (readField(field, candidate.text, candidate.classes) !== expected[field]) continue;
            if ((await readAt(page, candidate.path, field)) === expected[field]) {
                found[field] = candidate.path;
                break;
            }
        }
    }
    return FIELDS.every((field) => found[field] !== undefined) ? (found as Recipe) : undefined;
}

/** Calls the model for one page and charges the tokens it used to the ledger. */
async function extractWithModel(stagehand: StagehandInstance, page: BookPage): Promise<BookFields> {
    const before = usageOf(stagehand);
    ledger.modelCalls += 1;
    try {
        return await page.extract(EXTRACT_INSTRUCTION, BookSchema);
    } finally {
        const delta = diffUsage(usageOf(stagehand), before);
        ledger.usage = sumUsage(ledger.usage, delta);
        ledger.costUsd += costOf(delta);
    }
}

async function handleBook(ctx: StagehandCrawlingContext, url: string, category: string): Promise<void> {
    const { page, stagehand, log } = ctx;

    for (const recipe of recipes) {
        const book = await readWithRecipe(page, recipe);
        if (book !== undefined) {
            extracted.set(url, { ...book, url, category, method: 'layout' });
            return;
        }
    }

    if (ledger.costUsd >= BUDGET_USD) {
        unextracted.set(url, {
            url,
            category,
            reason: `page matches no learned layout and the model budget is spent (${usd(ledger.costUsd)} of ${usd(BUDGET_USD)})`,
        });
        return;
    }

    let book: BookFields;
    try {
        book = await extractWithModel(stagehand, page);
    } catch (error) {
        unextracted.set(url, { url, category, reason: `model extraction failed: ${errorText(error)}` });
        return;
    }
    extracted.set(url, { ...book, url, category, method: 'model' });

    const recipe = await learnRecipe(page, book);
    if (recipe === undefined) {
        log.warning(`No layout learned from ${url}; the next book without a learned layout will use the model.`);
    } else {
        recipes.push(recipe);
        log.info(`Learned layout ${recipes.length} from ${url}`);
    }
}

const crawler = new StagehandCrawler({
    stagehandOptions: { env: 'LOCAL', model: MODEL, verbose: 0, selfHeal: false },
    maxConcurrency: 1,
    maxRequestRetries: 1,
    requestHandler: async (ctx) => {
        const { request, page, enqueueLinks, log } = ctx;
        switch (request.label) {
            case 'HOME': {
                const categoryUrls = await page.$$eval('.side_categories ul ul a', (anchors) =>
                    anchors.map((anchor) => new URL(anchor.getAttribute('href') ?? '', document.baseURI).href),
                );
                const firstCategories = categoryUrls.slice(0, CATEGORY_COUNT);
                if (firstCategories.length < CATEGORY_COUNT) {
                    log.warning(`Only ${firstCategories.length} categories found in the side list`);
                }
                await enqueueLinks({ urls: firstCategories, label: 'CATEGORY' });
                return;
            }
            case 'CATEGORY': {
                const category = (await page.locator('.page-header h1').textContent())?.trim() ?? 'unknown';
                const bookUrls = await page.$$eval('article.product_pod h3 a', (anchors) =>
                    anchors.map((anchor) => new URL(anchor.getAttribute('href') ?? '', document.baseURI).href),
                );
                for (const url of bookUrls) listed.set(url, category);
                await enqueueLinks({ urls: bookUrls, label: 'BOOK', userData: { category } });
                await enqueueLinks({ selector: 'li.next a', label: 'CATEGORY' });
                return;
            }
            case 'BOOK': {
                const url = request.loadedUrl ?? request.url;
                const category = String(request.userData.category ?? 'unknown');
                await handleBook(ctx, url, category);
                return;
            }
            default:
                log.warning(`Unknown request label for ${request.url}`);
        }
    },
    failedRequestHandler: async ({ request }) => {
        if (request.label !== 'BOOK') return;
        const url = request.loadedUrl ?? request.url;
        const lastError = request.errorMessages.slice(-1)[0] ?? 'unknown error';
        unextracted.set(url, {
            url,
            category: String(request.userData.category ?? 'unknown'),
            reason: `request failed after retries: ${lastError}`,
        });
    },
});

await crawler.run([{ url: START_URL, label: 'HOME' }]);

for (const [url, category] of listed) {
    if (!extracted.has(url) && !unextracted.has(url)) {
        unextracted.set(url, { url, category, reason: 'listed on a category page but not reached by the crawl' });
    }
}

const { usage } = ledger;
const extractedBy = (method: Method): number => [...extracted.values()].filter((book) => book.method === method).length;
const lines = [
    `Books listed: ${listed.size}`,
    `Extracted: ${extracted.size} (${extractedBy('layout')} from a learned layout, ${extractedBy('model')} by the model)`,
    `Layouts learned: ${recipes.length}; model calls: ${ledger.modelCalls}`,
    `Left unextracted: ${unextracted.size}`,
    ...[...unextracted.values()].map((item) => `  - ${item.url} [${item.category}]: ${item.reason}`),
    `Tokens: prompt ${usage.promptTokens}, cached input ${usage.cachedInputTokens}, completion ${usage.completionTokens}, reasoning ${usage.reasoningTokens}`,
    `Cost: ${usd(ledger.costUsd)} of the ${usd(BUDGET_USD)} budget (${MODEL}, rates in PRICE_PER_MILLION_TOKENS)`,
];
console.log(lines.join('\n'));
