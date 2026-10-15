import { AdaptivePlaywrightCrawler, Dataset } from 'crawlee';
import type { Page } from 'playwright';

type Version = 'plain' | 'js' | 'scroll';

type Quote = { text: string; author: string; tags: string[] };

type StoredQuote = Quote & { site: Version; source: string };

type ScrollPage = { has_next: boolean; quotes: unknown[] };

type ScrollCapture = { pages: ScrollPage[] };

const SITES: Record<Version, string> = {
    plain: 'https://quotes.toscrape.com/',
    js: 'https://quotes.toscrape.com/js/',
    scroll: 'https://quotes.toscrape.com/scroll',
};

const RENDER_WAIT_MS = 15_000;
const SCROLL_STEP_WAIT_MS = 10_000;
const MAX_SCROLL_STEPS = 50;
const SCROLL_TO_BOTTOM =
    'window.scrollTo(0, Math.max(document.body.scrollHeight, document.documentElement.scrollHeight))';

// JSON responses captured while a scroll page loads, keyed by its browser page.
const scrollCaptures = new WeakMap<Page, ScrollCapture>();

function isVersion(value: unknown): value is Version {
    return value === 'plain' || value === 'js' || value === 'scroll';
}

function versionOf(userData: Record<string, unknown>): Version {
    const version = userData.version;
    if (!isVersion(version)) {
        throw new Error(`Request without a known site version: ${JSON.stringify(userData)}`);
    }
    return version;
}

// The page exists only in browser runs; reading it in a plain HTTP run throws.
function browserPageOf(context: object): Page | undefined {
    try {
        return (context as { page?: Page }).page;
    } catch {
        return undefined;
    }
}

async function blockImagesAndFonts(page: Page): Promise<void> {
    await page.route('**/*', (route) => {
        const type = route.request().resourceType();
        return type === 'image' || type === 'font' ? route.abort() : route.continue();
    });
}

function isScrollPage(body: unknown): body is ScrollPage {
    return (
        typeof body === 'object' &&
        body !== null &&
        'has_next' in body &&
        typeof body.has_next === 'boolean' &&
        'quotes' in body &&
        Array.isArray(body.quotes)
    );
}

// Attached before navigation so that the first JSON page is not missed.
function captureQuoteJson(page: Page): void {
    const capture: ScrollCapture = { pages: [] };
    scrollCaptures.set(page, capture);
    page.on('response', (response) => {
        const contentType = response.headers()['content-type'] ?? '';
        if (!contentType.includes('application/json')) {
            return;
        }
        response.json().then(
            (body: unknown) => {
                if (isScrollPage(body)) {
                    capture.pages.push(body);
                }
            },
            () => undefined,
        );
    });
}

function checkedQuote(raw: Quote): Quote {
    const text = raw.text.trim();
    const author = raw.author.trim();
    if (text === '' || author === '') {
        throw new Error(`Quote without text or author: ${JSON.stringify(raw)}`);
    }
    return { text, author, tags: raw.tags.map((tag) => tag.trim()).filter((tag) => tag !== '') };
}

function authorName(author: unknown): string {
    if (typeof author === 'string') {
        return author;
    }
    if (typeof author === 'object' && author !== null && 'name' in author && typeof author.name === 'string') {
        return author.name;
    }
    return '';
}

function quoteFromJson(raw: unknown): Quote {
    if (typeof raw !== 'object' || raw === null) {
        throw new Error(`Malformed quote in JSON: ${JSON.stringify(raw)}`);
    }
    const text = 'text' in raw && typeof raw.text === 'string' ? raw.text : '';
    const author = 'author' in raw ? authorName(raw.author) : '';
    const tags = 'tags' in raw && Array.isArray(raw.tags) ? raw.tags.filter((tag): tag is string => typeof tag === 'string') : [];
    return checkedQuote({ text, author, tags });
}

async function waitFor(condition: () => boolean, timeoutMs: number): Promise<boolean> {
    const deadline = Date.now() + timeoutMs;
    while (!condition()) {
        if (Date.now() >= deadline) {
            return false;
        }
        await new Promise((resolve) => setTimeout(resolve, 100));
    }
    return true;
}

// Scrolls until the captured JSON reports no further page; the quotes come from those responses only.
async function scrollQuotes(page: Page | undefined): Promise<Quote[]> {
    if (page === undefined) {
        return [];
    }
    const capture = scrollCaptures.get(page);
    if (capture === undefined) {
        throw new Error('The scroll page has no JSON capture');
    }
    for (let step = 0; !capture.pages.some((scrollPage) => !scrollPage.has_next); step++) {
        if (step >= MAX_SCROLL_STEPS) {
            throw new Error(`Scrolling did not reach the last JSON page after ${step} steps`);
        }
        const seen = capture.pages.length;
        await page.evaluate(SCROLL_TO_BOTTOM);
        await page.mouse.wheel(0, 2000);
        const loaded = await waitFor(() => capture.pages.length > seen, SCROLL_STEP_WAIT_MS);
        if (!loaded) {
            throw new Error(`Scrolling stopped after ${seen} JSON pages`);
        }
    }
    return capture.pages.flatMap((scrollPage) => scrollPage.quotes).map(quoteFromJson);
}

const crawler = new AdaptivePlaywrightCrawler({
    requestHandlerTimeoutSecs: 180,
    // Every page starts as a plain HTTP fetch. The result checker sends pages without quotes to a browser.
    // The default predictor opens a browser for unseen URLs, so it is replaced here.
    renderingTypePredictor: {
        initialize: async () => undefined,
        predict: () => ({ renderingType: 'static', detectionProbabilityRecommendation: 0 }),
        storeResult: () => undefined,
    },
    preNavigationHooks: [
        async (context) => {
            const page = browserPageOf(context);
            if (page === undefined) {
                return;
            }
            await blockImagesAndFonts(page);
            if (versionOf(context.request.userData) === 'scroll') {
                captureQuoteJson(page);
            }
        },
    ],
    resultChecker: (result) => result.datasetItems.length > 0,
    requestHandler: async (context) => {
        const { request, pushData, enqueueLinks } = context;
        const version = versionOf(request.userData);
        const page = browserPageOf(context);

        let quotes: Quote[];
        if (version === 'scroll') {
            quotes = await scrollQuotes(page);
        } else {
            if (page !== undefined) {
                await page.waitForSelector('div.quote', { timeout: RENDER_WAIT_MS }).catch(() => undefined);
            }
            const $ = await context.parseWithCheerio();
            quotes = $('div.quote')
                .toArray()
                .map((element) => {
                    const quote = $(element);
                    return checkedQuote({
                        text: quote.find('span.text').first().text(),
                        author: quote.find('small.author').first().text(),
                        tags: quote
                            .find('a.tag')
                            .toArray()
                            .map((tag) => $(tag).text()),
                    });
                });
        }

        if (quotes.length === 0) {
            if (page === undefined) {
                // The plain HTTP pass found nothing: the result checker sends this page to a browser.
                return;
            }
            throw new Error(`No quotes found in the browser on ${request.url}`);
        }

        for (const quote of quotes) {
            await pushData({ site: version, source: request.url, ...quote });
        }

        if (version !== 'scroll') {
            await enqueueLinks({ selector: 'li.next > a', userData: { version } });
        }
    },
});

await crawler.run([
    { url: SITES.plain, userData: { version: 'plain' } },
    { url: SITES.js, userData: { version: 'js' } },
    { url: SITES.scroll, userData: { version: 'scroll' } },
]);

const dataset = await Dataset.open<StoredQuote>();
const { items } = await dataset.getData();

// Each quote is recorded once per site version.
const unique = new Map<string, StoredQuote>();
for (const item of items) {
    const key = JSON.stringify([item.site, item.author, item.text]);
    if (!unique.has(key)) {
        unique.set(key, item);
    }
}

const quotes = Array.from(unique.values());
for (const site of ['plain', 'js', 'scroll'] as const) {
    console.log(`${site}: ${quotes.filter((quote) => quote.site === site).length} quotes`);
}
console.log(JSON.stringify(quotes, null, 2));
