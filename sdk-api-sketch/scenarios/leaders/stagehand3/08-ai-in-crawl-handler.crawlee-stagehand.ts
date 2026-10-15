// Golden scenario 08 on Crawlee 3.18.2's StagehandCrawler and Stagehand 3.7.3.
// Stagehand 3 caches `act` and the agent (`cacheDir`), never `extract`, which asks the model on every
// page. Learning once per page type follows Stagehand's documented "observe, then replay with no
// inference", here for reads: one `observe` per field finds its element, the program keeps the
// selectors per label and reads each later book with Playwright, observing again when a read
// fails. Stagehand counts tokens per instance (one per browser) and prices nothing, and nothing
// caps the spend: the program does both.
import {
  Dataset,
  StagehandCrawler,
  createStagehandRouter,
  type Stagehand,
  type StagehandPage,
} from "@crawlee/stagehand";
import { z } from "zod";

const BUDGET_USD = 0.5;
const USD_PER_TOKEN = { input: 0.25 / 1e6, output: 2 / 1e6 }; // gpt-5-mini's list price, by hand
const RATINGS = ["One", "Two", "Three", "Four", "Five"];

const Book = z.object({
  title: z.string().trim().min(1),
  price: z
    .string()
    .transform((text) => /\d+\.\d{2}/.exec(text)?.[0])
    .pipe(z.string()),
  inStock: z
    .string()
    .transform((text) => Number(/(\d+) available/.exec(text)?.[1]))
    .pipe(z.number().int().min(0)),
  rating: z
    .string()
    .transform((cls) => RATINGS.findIndex((word) => cls.split(" ").includes(word)) + 1)
    .pipe(z.number().int().min(1).max(5)),
});
type Book = z.infer<typeof Book>;

const fields = {
  title: "the book's title",
  price: "the book's price",
  inStock: "the availability line with the number in stock",
  rating: "the star rating of the book",
} as const;
type Selectors = Record<keyof typeof fields, string>;

const learned = new Map<string, Selectors>();
const instances = new Set<Stagehand>();
const unextracted: { url: string; why: string }[] = [];

async function spent(): Promise<{ tokens: number; usd: number }> {
  let input = 0;
  let output = 0;
  for (const stagehand of instances) {
    const metrics = await stagehand.metrics;
    input += metrics.totalPromptTokens;
    output += metrics.totalCompletionTokens;
  }
  return {
    tokens: input + output,
    usd: input * USD_PER_TOKEN.input + output * USD_PER_TOKEN.output,
  };
}

async function learn(stagehand: Stagehand, page: StagehandPage): Promise<Selectors> {
  const selectors: Partial<Selectors> = {};
  for (const [field, instruction] of Object.entries(fields) as [keyof Selectors, string][]) {
    const [action] = await stagehand.observe(`find ${instruction}`, { page });
    if (action === undefined) throw new Error(`no element for ${field}`);
    selectors[field] = action.selector; // an XPath from the document root
  }
  return selectors as Selectors;
}

async function read(page: StagehandPage, selectors: Selectors): Promise<Book> {
  const text = (selector: string) => page.locator(selector).textContent({ timeout: 2_000 });
  return Book.parse({
    title: await text(selectors.title),
    price: await text(selectors.price),
    inStock: await text(selectors.inStock),
    rating: await page.locator(selectors.rating).getAttribute("class", { timeout: 2_000 }),
  });
}

const router = createStagehandRouter();

router.addDefaultHandler(async ({ enqueueLinks }) => {
  await enqueueLinks({ selector: ".side_categories ul ul a", label: "category", limit: 3 });
});

router.addHandler("category", async ({ enqueueLinks }) => {
  await enqueueLinks({ selector: "article.product_pod h3 a", label: "book" });
  await enqueueLinks({ selector: "li.next a", label: "category" });
});

router.addHandler("book", async ({ page, request, stagehand }) => {
  instances.add(stagehand);
  const known = learned.get("book");
  let book = known === undefined ? null : await read(page, known).catch(() => null);
  if (book === null) {
    // Nothing learned yet, or the page no longer matches what was: back to the model, within budget.
    if ((await spent()).usd >= BUDGET_USD) {
      unextracted.push({ url: request.url, why: `budget of ${BUDGET_USD} USD reached` });
      return;
    }
    const selectors = await learn(stagehand, page);
    learned.set("book", selectors);
    book = await read(page, selectors);
  }
  await Dataset.pushData({ ...book, url: request.loadedUrl ?? request.url });
});

const crawler = new StagehandCrawler({
  stagehandOptions: { env: "LOCAL", model: "openai/gpt-5-mini" },
  requestHandler: router,
  // After `maxRequestRetries` (3): the errors are kept as text on the request.
  async failedRequestHandler({ request }) {
    unextracted.push({ url: request.url, why: request.errorMessages.at(-1) ?? "failed" });
  },
});

await crawler.run(["https://books.toscrape.com/"]);
for (const { url, why } of unextracted) console.log("unextracted", url, why);
const { tokens, usd } = await spent();
console.log(`model use: ${tokens} tokens, about ${usd.toFixed(4)} USD`);
