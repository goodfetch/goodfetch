// Golden scenario 01, "Fill a dependent form and read the results": the leader, Playwright 1.64.1
// (TypeScript), as a library script.
import { chromium } from "playwright";

interface Quote {
  text: string;
  author: string;
  tag: string;
}

async function searchQuotes(author: string, tag: string): Promise<Quote[]> {
  const browser = await chromium.launch();
  try {
    const page = await browser.newPage();
    await page.goto("https://quotes.toscrape.com/search.aspx");
    // Choosing the author posts the form back; selectOption on the tag waits for its option to appear.
    await page.getByLabel("Author").selectOption(author);
    await page.getByLabel("Tag").selectOption(tag);
    const responsePromise = page.waitForResponse("**/filter.aspx");
    await page.getByRole("button", { name: "Search" }).click();
    const response = await responsePromise;
    if (!response.ok()) throw new Error(`Search: filter.aspx answered ${response.status()}`);
    await page.waitForLoadState();

    const quotes: Quote[] = [];
    for (const row of await page.locator(".results .quote").all()) {
      quotes.push({
        text: await row.locator(".content").innerText(),
        author: await row.locator(".author").innerText(),
        tag: await row.locator(".tag").innerText(),
      });
    }
    if (quotes.length === 0) {
      // The site heads its results "Results" only above a match: that heading over no quote is a changed layout.
      await page.getByRole("heading", { name: "Results" }).waitFor({ state: "hidden" });
    }
    return quotes;
  } finally {
    await browser.close();
  }
}

const quotes = await searchQuotes("Albert Einstein", "inspirational");
if (quotes.length === 0) console.log("No quote matches Albert Einstein and inspirational.");
for (const quote of quotes) console.log(`${quote.text} (${quote.author}, ${quote.tag})`);
