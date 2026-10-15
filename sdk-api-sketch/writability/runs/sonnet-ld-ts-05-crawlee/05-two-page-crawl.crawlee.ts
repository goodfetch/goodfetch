import { CheerioCrawler, Dataset } from 'crawlee';
import type { SkippedRequestReason } from '@crawlee/core';

const START_URL = 'https://books.toscrape.com/';
const DATASET_NAME = 'books';
const EXPORT_KEY = 'books';

const LABEL_CATEGORY = 'CATEGORY';
const LABEL_BOOK = 'BOOK';

const RATINGS: Record<string, number> = { One: 1, Two: 2, Three: 3, Four: 4, Five: 5 };

interface BookItem {
    title: string;
    price: number;
    inStock: number;
    rating: number;
    url: string;
    category: string;
}

let succeeded = 0;
const failed: { url: string; reason: string }[] = [];
const skipped: { url: string; reason: SkippedRequestReason }[] = [];

const dataset = await Dataset.open<BookItem>(DATASET_NAME);

function categoryOf(userData: Record<string, unknown>): string {
    const category = userData.category;
    if (typeof category !== 'string' || category === '') {
        throw new Error('The category name was not passed to this page');
    }
    return category;
}

const crawler = new CheerioCrawler({
    maxRequestRetries: 3,
    onSkippedRequest: ({ url, reason }) => {
        skipped.push({ url, reason });
    },
    requestHandler: async ({ $, request, enqueueLinks }) => {
        if (request.label === LABEL_BOOK) {
            const category = categoryOf(request.userData);
            const title = $('div.product_main h1').first().text().trim();
            const priceMatch = /\d+(?:\.\d+)?/.exec($('div.product_main p.price_color').first().text());
            const stockMatch = /(\d+)\s+available/.exec($('div.product_main p.availability').first().text());
            const ratingClass = ($('div.product_main p.star-rating').first().attr('class') ?? '')
                .split(/\s+/)
                .find((name) => name in RATINGS);

            if (title === '' || priceMatch === null || stockMatch === null || ratingClass === undefined) {
                throw new Error('The book page is missing its title, price, stock or rating');
            }

            await dataset.pushData({
                title,
                price: Number.parseFloat(priceMatch[0]),
                inStock: Number.parseInt(stockMatch[1] ?? '', 10),
                rating: RATINGS[ratingClass] ?? 0,
                url: request.loadedUrl ?? request.url,
                category,
            });
        } else if (request.label === LABEL_CATEGORY) {
            const category = categoryOf(request.userData);
            await enqueueLinks({
                selector: 'article.product_pod h3 a',
                strategy: 'same-hostname',
                label: LABEL_BOOK,
                userData: { category },
            });
            await enqueueLinks({
                selector: 'li.next a',
                strategy: 'same-hostname',
                label: LABEL_CATEGORY,
                userData: { category },
            });
        } else {
            // Home page: one request per category in the side list, except the top-level "Books".
            const base = request.loadedUrl ?? request.url;
            const categories: { name: string; url: string }[] = [];
            $('div.side_categories ul.nav-list > li > ul > li > a').each((_index, element) => {
                const name = $(element).text().trim();
                const href = $(element).attr('href');
                if (name !== '' && name !== 'Books' && href !== undefined) {
                    categories.push({ name, url: new URL(href, base).href });
                }
            });
            for (const { name, url } of categories) {
                await enqueueLinks({
                    urls: [url],
                    strategy: 'same-hostname',
                    label: LABEL_CATEGORY,
                    userData: { category: name },
                });
            }
        }
        succeeded += 1;
    },
    failedRequestHandler: async ({ request }, error) => {
        failed.push({ url: request.url, reason: error.message });
    },
});

await crawler.run([START_URL]);

await dataset.exportToJSON(EXPORT_KEY);

console.log(`Succeeded: ${succeeded}`);
console.log(`Failed: ${failed.length}`);
for (const { url, reason } of failed) {
    console.log(`  failed ${url}: ${reason}`);
}
console.log(`Skipped: ${skipped.length}`);
for (const { url, reason } of skipped) {
    console.log(`  skipped ${url}: ${reason}`);
}
