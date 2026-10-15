import { chromium } from "playwright";
import { Stagehand } from "@browserbasehq/stagehand";
import { z } from "zod";

// Stagehand can only act on a page of a browser it is connected to over CDP,
// so the program's Chromium exposes its DevTools endpoint on a local port.
const debugPort = 9222;
const browser = await chromium.launch({
  args: [`--remote-debugging-port=${debugPort}`],
});
const page = await browser.newPage();
await page.goto("https://books.toscrape.com/");
await page.getByRole("link", { name: "Travel" }).click();

// AI step: extract every listed book from the page the program drives.
// It attaches to the existing browser and never closes, navigates or opens
// pages; any failure is logged and the program carries on.
let stagehand: Stagehand | undefined;
try {
  const versionResponse = await fetch(
    `http://127.0.0.1:${debugPort}/json/version`,
  );
  const { webSocketDebuggerUrl } = (await versionResponse.json()) as {
    webSocketDebuggerUrl: string;
  };
  stagehand = new Stagehand({
    env: "LOCAL",
    model: "openai/gpt-4.1-mini",
    localBrowserLaunchOptions: { cdpUrl: webSocketDebuggerUrl },
  });
  await stagehand.init();
  const { books } = await stagehand.extract(
    "Extract every book listed on this page with its title and price.",
    z.object({
      books: z.array(z.object({ title: z.string(), price: z.string() })),
    }),
    { page },
  );
  console.log(books);
} catch (error) {
  console.error("AI extraction failed:", error);
}
// Closing Stagehand only drops its CDP connection; the browser stays open.
await stagehand?.close().catch(() => undefined);

await page.locator("article.product_pod h3 a").first().click();
console.log(await page.locator(".product_main h1").textContent());
await browser.close();
