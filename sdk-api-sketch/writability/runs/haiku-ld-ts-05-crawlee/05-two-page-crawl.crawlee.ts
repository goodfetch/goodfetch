import { CheerioCrawler, Dataset } from 'crawlee';

const START_URL = 'https://books.toscrape.com/';
const DATASET_NAME = 'books';
const EXPORT_KEY = 'books.json';

type BookItem = {
    title: string;
    price: number;
    inStock: number;
    rating: number;
    url: string;
    category: string;
};

interface OutcomeRecord {
    url: string;
    reason: string;
}

const RATING_WORDS = new Map<string, number>([
    ['One', 1],
    ['Two', 2],
    ['Three', 3],
    ['Four', 4],
    ['Five', 5],
]);

async function main(): Promise<void> {
    const books = await Dataset.open<BookItem>(DATASET_NAME);
    const failed: OutcomeRecord[] = [];
    const skipped: OutcomeRecord[] = [];

    const crawler = new CheerioCrawler({
        respectRobotsTxtFile: true,
        onSkippedRequest: ({ url, reason }) => {
            skipped.push({ url, reason });
        },
        failedRequestHandler: ({ request }, error) => {
            failed.push({ url: request.url, reason: error.message });
        },
        requestHandler: async ({ request, $, enqueueLinks }) => {
            if (request.label === 'CATEGORY') {
                const category = $('.page-header h1').first().text().trim();
                if (category === '') {
                    throw new Error('category page has no heading to name it');
                }
                await enqueueLinks({
                    selector: 'article.product_pod h3 a',
                    label: 'BOOK',
                    userData: { category },
                    strategy: 'same-hostname',
                });
                await enqueueLinks({
                    selector: 'li.next a',
                    label: 'CATEGORY',
                    userData: { category },
                    strategy: 'same-hostname',
                });
                return;
            }

            if (request.label === 'BOOK') {
                const category: unknown = request.userData.category;
                if (typeof category !== 'string') {
                    throw new Error('book page was not reached from a category page');
                }

                const title = $('.product_main h1').first().text().trim();
                if (title === '') {
                    throw new Error('book page has no title');
                }

                const priceMatch = /(\d+(?:\.\d+)?)/.exec($('.product_main .price_color').first().text());
                if (priceMatch === null) {
                    throw new Error('book page has no price');
                }
                const price = Number(priceMatch[1]);

                const availability = $('.product_main .instock.availability').first().text().replace(/\s+/g, ' ').trim();
                let inStock: number;
                const stockMatch = /\((\d+) available\)/.exec(availability);
                if (stockMatch !== null) {
                    inStock = Number.parseInt(stockMatch[1], 10);
                } else if (/out of stock/i.test(availability)) {
                    inStock = 0;
                } else {
                    throw new Error(`book page has no stock count ("${availability}")`);
                }

                const ratingWord = $('.product_main p.star-rating')
                    .first()
                    .attr('class')
                    ?.split(/\s+/)
                    .find((name) => RATING_WORDS.has(name));
                const rating = ratingWord === undefined ? undefined : RATING_WORDS.get(ratingWord);
                if (rating === undefined) {
                    throw new Error('book page has no star rating');
                }

                await books.pushData({
                    title,
                    price,
                    inStock,
                    rating,
                    url: request.loadedUrl ?? request.url,
                    category,
                });
                return;
            }

            // The home page: follow every category link in the side list except the top-level "Books" one.
            await enqueueLinks({
                selector: '.side_categories ul.nav-list > li > ul > li > a',
                label: 'CATEGORY',
                strategy: 'same-hostname',
            });
        },
    });

    const stats = await crawler.run([START_URL]);
    await books.exportToJSON(EXPORT_KEY);

    console.log(`Succeeded pages: ${stats.requestsFinished}`);
    console.log(`Failed pages: ${failed.length}`);
    for (const record of failed) {
        console.log(`  failed ${record.url}: ${record.reason}`);
    }
    console.log(`Skipped pages: ${skipped.length}`);
    for (const record of skipped) {
        console.log(`  skipped ${record.url}: ${record.reason}`);
    }
    console.log(`Dataset "${DATASET_NAME}" exported to key-value store file "${EXPORT_KEY}".`);
}

main().catch((error: unknown) => {
    console.error(error);
    process.exitCode = 1;
});
