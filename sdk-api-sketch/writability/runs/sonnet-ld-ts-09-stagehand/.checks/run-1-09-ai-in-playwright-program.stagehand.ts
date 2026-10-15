import { chromium } from "playwright";
import { Stagehand } from "@browserbasehq/stagehand";
import { z } from "zod";

const debuggingPort = 9222;

// The only change to the launch: expose a CDP endpoint so Stagehand can attach to this browser.
const browser = await chromium.launch({
  args: [`--remote-debugging-port=${debuggingPort}`],
});
const page = await browser.newPage();
await page.goto("https://books.toscrape.com/");
await page.getByRole("link", { name: "Travel" }).click();

// Added step: extract the title and price of every listed book with a language model.
// Stagehand only attaches to the browser; it never launches, closes or navigates anything.
let stagehand: Stagehand | undefined;
try {
  const version = (await (
    await fetch(`http://127.0.0.1:${debuggingPort}/json/version`)
  ).json()) as { webSocketDebuggerUrl: string };

  stagehand = new Stagehand({
    env: "LOCAL",
    model: "anthropic/claude-sonnet-4-5",
    localBrowserLaunchOptions: { cdpUrl: version.webSocketDebuggerUrl },
    disableAPI: true,
  });
  await stagehand.init();

  const books = await stagehand.extract(
    "Extract the title and price of every book listed on the page.",
    z.object({
      books: z.array(
        z.object({
          title: z.string().describe("the full title of the book"),
          price: z.string().describe("the price of the book, with its currency symbol"),
        }),
      ),
    }),
    { page },
  );
  console.log(books.books);
} catch (error) {
  console.error("Extraction failed, continuing without it:", error);
} finally {
  // Closing Stagehand only drops its CDP connection to the attached browser.
  await stagehand?.close().catch(() => {});
}

await page.locator("article.product_pod h3 a").first().click();
console.log(await page.locator(".product_main h1").textContent());
await browser.close();
