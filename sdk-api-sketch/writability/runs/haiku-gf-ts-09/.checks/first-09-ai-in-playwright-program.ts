import { chromium, type Page } from "playwright";
import { Collie } from "@goodfetch/collie";
import { z } from "zod";

const booksSchema = z.object({
  books: z.array(z.object({ title: z.string(), price: z.string() })),
});

type Book = z.infer<typeof booksSchema>["books"][number];

// Borrows the program's own Playwright page for one AI extraction. Ending the borrow only releases the
// page: the Playwright page, its context and its browser stay open for the program's own code.
async function extractBooks(page: Page): Promise<Book[]> {
  await using collie = await Collie.connect();
  const run = await collie.start();
  try {
    const borrowed = await run.borrow(page);
    try {
      const result = await borrowed.extract(
        "List every book on this page with its title and its price.",
        booksSchema,
      );
      return result.output.books;
    } finally {
      await borrowed.close();
    }
  } finally {
    await run.finish();
  }
}

const browser = await chromium.launch();
const page = await browser.newPage();
await page.goto("https://books.toscrape.com/");
await page.getByRole("link", { name: "Travel" }).click();

try {
  await page.waitForLoadState("load");
  const books = await extractBooks(page);
  console.log(JSON.stringify(books, null, 2));
} catch (error) {
  console.error("Extracting the books with a language model failed; continuing:", error);
}

await page.locator("article.product_pod h3 a").first().click();
console.log(await page.locator(".product_main h1").textContent());
await browser.close();
