// Golden scenario 09, "Add an AI step to an existing Playwright program" (ADRs 0056, 0078, 0089, 0091).
import { chromium } from "playwright";
import { z } from "zod";
import { Collie } from "@goodfetch/collie";

const Listed = z.array(z.object({ title: z.string(), price: z.number() })); // pounds

// The existing program, unchanged: it launches Chromium as Playwright does by default (over a pipe, no port).
const browser = await chromium.launch();
const page = await browser.newPage();
await page.goto("https://books.toscrape.com/");
await page.getByRole("link", { name: "Travel" }).click();

// The added step. It borrows the page through the program's own Playwright connection (ADR 0056's
// relay), and leaving the block ends the borrow: the page stays open, where it was (ADR 0078).
try {
  await using collie = await Collie.connect(); // Anthropic's key: env:ANTHROPIC_API_KEY, sent only to Anthropic
  await using run = await collie.start();
  await using borrowed = await run.borrow(page);
  const { output: books } = await borrowed
    .locator("ol.row")
    .extract("every book listed: title and price", Listed, {
      model: "anthropic/claude-sonnet-4-5",
    });
  console.log(books);
  await run.finish();
} catch (error) {
  // A typed GoodfetchError (`model_unavailable`, `extraction_incomplete`, `borrow_requires_cdp`, …) with its `fix`.
  console.error("The AI step failed; the program carries on:", error);
}

// The existing program carries on with its page and closes its own browser.
await page.locator("article.product_pod h3 a").first().click();
console.log(await page.locator(".product_main h1").textContent());
await browser.close();
