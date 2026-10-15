// Golden scenario 09, "Add an AI step to an existing Playwright program", on Stagehand 3.7.3: the
// last major that takes a Playwright page. Stagehand attaches over CDP, so the existing program must
// open a debugging port when it launches Chromium.
import { Stagehand } from "@browserbasehq/stagehand";
import { chromium } from "playwright";
import { z } from "zod";

// The existing program, but for the port its launch now opens.
const browser = await chromium.launch({ args: ["--remote-debugging-port=9222"] });
const page = await browser.newPage();
await page.goto("https://books.toscrape.com/");
await page.getByRole("link", { name: "Travel" }).click();

// The added step. Stagehand 3.7.3 opens `cdpUrl` as a WebSocket, so it needs the browser's own address.
let stagehand: Stagehand | undefined;
try {
  const version = await fetch("http://127.0.0.1:9222/json/version");
  const { webSocketDebuggerUrl } = (await version.json()) as { webSocketDebuggerUrl: string };
  stagehand = new Stagehand({
    env: "LOCAL",
    localBrowserLaunchOptions: { cdpUrl: webSocketDebuggerUrl },
    model: "anthropic/claude-sonnet-4-5", // reads ANTHROPIC_API_KEY
  });
  await stagehand.init();
  const { books } = await stagehand.extract(
    "every book listed: title and price",
    z.object({ books: z.array(z.object({ title: z.string(), price: z.number() })) }),
    { page },
  );
  console.log(books);
} catch (error) {
  console.error("The AI step failed; the program carries on:", error);
} finally {
  await stagehand?.close(); // on a `cdpUrl` attach, closes its socket and leaves the browser running
}

// The existing program carries on with its page and closes its own browser.
await page.locator("article.product_pod h3 a").first().click();
console.log(await page.locator(".product_main h1").textContent());
await browser.close();
