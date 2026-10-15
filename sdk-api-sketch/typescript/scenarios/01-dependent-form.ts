// Golden scenario 01, "Fill a dependent form and read the results" (Collie, TypeScript).
import { Collie } from "@goodfetch/collie";

/** A quote the search shows. */
export interface Quote {
  readonly text: string;
  readonly author: string;
  readonly tag: string;
}

/** Every quote the search shows for `author` and `tag`: an empty list when none matches. */
export async function searchQuotes(author: string, tag: string): Promise<Quote[]> {
  await using collie = await Collie.connect();
  // The Page check judges every page: one without the search form (an error page, a changed site)
  // fails the call that reached it with `PageCheckFailedError`, so an empty list is never a broken page.
  await using run = await collie.start({
    settings: { session: { pageCheck: { expectedPage: { selector: "form[name='filterform']" } } } },
  });
  const page = await run.newPage();

  await page.goto("https://quotes.toscrape.com/search.aspx");
  // Choosing the author posts the form back, and the action waits for that navigation to commit.
  await page.getByLabel("Author").selectOption(author);
  // The guard waits for the reloaded list to offer the tag. Each failure is typed and names the call,
  // its locator and what it saw (here the options offered), with the timeout's setting.
  await page.getByLabel("Tag").selectOption(tag);
  await page.getByRole("button", { name: "Search" }).click();
  await page.waitForLoadState(); // the results' document has loaded before it is read

  // An empty list is an answer, not an error (G1); the reads run concurrently, one round trip each.
  const rows = await page.locator(".results .quote").all();
  const quotes = await Promise.all(
    rows.map(async (row) => ({
      text: await row.locator(".content").textContent(),
      author: await row.locator(".author").textContent(),
      tag: await row.locator(".tag").textContent(),
    })),
  );
  if (quotes.length === 0) {
    // The site heads its results "Results" only above a match: that heading over no quote is a changed layout.
    await page.getByRole("heading", { name: "Results" }).waitFor({ state: "hidden" });
  }
  await run.finish(); // closes the Run's Session and page; `await using` closes the rest on every path
  return quotes;
}

const quotes = await searchQuotes("Albert Einstein", "inspirational");
if (quotes.length === 0) console.log("No quote matches Albert Einstein and inspirational.");
for (const quote of quotes) console.log(`${quote.text} (${quote.author}, ${quote.tag})`);
