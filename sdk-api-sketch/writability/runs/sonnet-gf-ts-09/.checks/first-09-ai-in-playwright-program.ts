import { chromium } from "playwright";
import type { Page as PlaywrightPage } from "playwright";
import { Collie } from "@goodfetch/collie";
import { z } from "zod";

const books = z.object({
  books: z.array(z.object({ title: z.string(), price: z.string() })),
});

// Reads every listed book's title and price with a language model, on the page the program already
// drives. The page is only borrowed: ending the borrow never closes, navigates or takes over the
// browser, and any failure is reported here instead of thrown, so the program carries on.
async function extractBooks(playwrightPage: PlaywrightPage): Promise<void> {
  try {
    await using collie = await Collie.connect();
    await using run = await collie.start();
    const page = await run.borrow(playwrightPage);
    const { output } = await page.extract("every book listed on the page, with its title and its price", books);
    for (const book of output.books) {
      console.log(`${book.title}: ${book.price}`);
    }
    await run.finish();
  } catch (error) {
    console.error("AI extraction failed, carrying on:", error);
  }
}

const browser = await chromium.launch();
const page = await browser.newPage();
await page.goto("https://books.toscrape.com/");
await page.getByRole("link", { name: "Travel" }).click();
await extractBooks(page);
await page.locator("article.product_pod h3 a").first().click();
console.log(await page.locator(".product_main h1").textContent());
await browser.close();
