import { AdaptivePlaywrightCrawler } from 'crawlee';
import type { Response } from 'playwright';

type Version = 'html' | 'js' | 'scroll';

interface Quote {
    version: Version;
    text: string;
    author: string;
    tags: string[];
}

interface ApiQuote {
    text: string;
    tags: string[];
    author: { name: string };
}

interface ApiPage {
    has_next: boolean;
    page: number;
    quotes: ApiQuote[];
}

interface ScrollState {
    pages: Map<number, ApiPage>;
    pending: Promise<void>[];
}

const START_URLS = [
    'https://quotes.toscrape.com/',
    'https://quotes.toscrape.com/js/',
    'https://quotes.toscrape.com/scroll',
];

// JSON responses captured per request while its page is open in the browser.
const scrollStates = new Map<string, ScrollState>();

function versionOf(url: string): Version {
    const { pathname } = new URL(url);
    if (pathname.startsWith('/js')) return 'js';
    if (pathname.startsWith('/scroll')) return 'scroll';
    return 'html';
}

function isQuotesApi(url: string): boolean {
    const parsed = new URL(url);
    return parsed.pathname === '/api/quotes';
}

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

const crawler = new AdaptivePlaywrightCrawler({
    // Roughly all requests are checked for the rendering type until the predictor is confident.
    renderingTypeDetectionRatio: 0.1,
    maxRequestRetries: 3,
    // A page fetched over HTTP that produced no quotes is not a success: it is redone in the browser.
    resultChecker: (result) => result.datasetItems.length > 0,

    preNavigationHooks: [
        async ({ request, page: pageGetter }) => {
            // The page does not exist in the HTTP-only run: reading it throws there, so skip.
            let page;
            try {
                page = pageGetter;
            } catch {
                return;
            }
            if (!page) return;

            // No images or fonts in the browser.
            await page.route('**/*', async (route) => {
                const type = route.request().resourceType();
                if (type === 'image' || type === 'font') {
                    await route.abort();
                } else {
                    await route.continue();
                }
            });

            // Collect the JSON the scrolling version loads, from the start of navigation.
            if (versionOf(request.url) === 'scroll') {
                const state: ScrollState = { pages: new Map(), pending: [] };
                scrollStates.set(request.uniqueKey, state);
                page.on('response', (response: Response) => {
                    if (!isQuotesApi(response.url()) || !response.ok()) return;
                    state.pending.push(
                        (async () => {
                            const body = (await response.json()) as ApiPage;
                            state.pages.set(body.page, body);
                        })().catch(() => undefined),
                    );
                });
            }
        },
    ],

    async requestHandler({ request, page, parseWithCheerio, enqueueLinks, pushData, log }) {
        const version = versionOf(request.url);

        if (version === 'scroll') {
            // Reading `page` in the HTTP-only run throws, which sends the request to the browser.
            const state = scrollStates.get(request.uniqueKey);
            if (!state) throw new Error('No JSON capture for the scrolling page');
            try {
                const waitFor = async (condition: () => boolean, timeoutMs: number) => {
                    const deadline = Date.now() + timeoutMs;
                    while (!condition()) {
                        if (Date.now() > deadline) return false;
                        await sleep(200);
                    }
                    return true;
                };
                const lastPage = () => {
                    let last: ApiPage | undefined;
                    for (const p of state.pages.values()) {
                        if (!last || p.page > last.page) last = p;
                    }
                    return last;
                };

                if (!(await waitFor(() => state.pages.size > 0, 15000))) {
                    throw new Error('The scrolling page loaded no JSON');
                }
                while (lastPage()?.has_next) {
                    const before = state.pages.size;
                    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
                    if (!(await waitFor(() => state.pages.size > before, 15000))) {
                        throw new Error('Scrolling did not load the next JSON page');
                    }
                }
                await Promise.all(state.pending);

                const quotes: Quote[] = [...state.pages.values()]
                    .sort((a, b) => a.page - b.page)
                    .flatMap((p) => p.quotes)
                    .map((q) => ({ version, text: q.text, author: q.author.name, tags: q.tags }));
                log.info(`Scroll version: ${quotes.length} quotes from ${state.pages.size} JSON pages`);
                await pushData(quotes);
            } finally {
                scrollStates.delete(request.uniqueKey);
            }
            return;
        }

        // HTML and JS versions: the same handler runs over HTTP or in the browser.
        // Over HTTP this throws when there are no quotes in the HTML, which moves the request to the browser.
        const $ = await parseWithCheerio('.quote', 15000);
        const quotes: Quote[] = $('.quote')
            .toArray()
            .map((el) => {
                const quote = $(el);
                return {
                    version,
                    text: quote.find('.text').first().text().trim(),
                    author: quote.find('.author').first().text().trim(),
                    tags: quote
                        .find('.tags a.tag')
                        .toArray()
                        .map((tag) => $(tag).text().trim()),
                };
            });
        if (quotes.length === 0) {
            throw new Error(`No quotes on ${request.url}`);
        }
        await pushData(quotes);

        const next = $('li.next a').first().attr('href');
        if (next) {
            await enqueueLinks({ urls: [new URL(next, request.loadedUrl ?? request.url).href] });
        }
    },

    failedRequestHandler({ request, log }, error) {
        log.error(`Failed ${request.url}: ${error.message}`);
    },
});

await crawler.run(START_URLS);

const { items } = await crawler.getData();
const seen = new Set<string>();
const unique: Quote[] = [];
for (const item of items as unknown as Quote[]) {
    const key = JSON.stringify([item.version, item.text, item.author]);
    if (seen.has(key)) continue;
    seen.add(key);
    unique.push(item);
}

const counts: Record<Version, number> = { html: 0, js: 0, scroll: 0 };
for (const quote of unique) counts[quote.version]++;
console.log(JSON.stringify({ counts, quotes: unique }, null, 2));
