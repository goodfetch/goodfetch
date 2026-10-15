// Snippet for Time, waits, deadlines, cancellation and Limits (#467).
import { DeadlineExceededError, Goodfetch, type Logger } from "goodfetch";

declare const log: Logger; // the program's own logger

export async function searchQuotes(stop: AbortSignal): Promise<number> {
  // Limits on this connection's work: a program may set or lower them, never raise them.
  await using gf = await Goodfetch.connect({
    logger: log,
    limits: { price: { amount: "2.00", currency: "EUR" } },
  });
  // The whole Run may last two hours; past that it stops with a Checkpoint and can be resumed.
  await using run = await gf.collie.start({ timeout: 2 * 60 * 60_000, limits: { modelTokens: 200_000 } });
  const page = await run.newPage();

  await page.goto("https://quotes.toscrape.com/search.aspx", { timeout: 10_000 });
  // Choosing an author posts the form back to search.aspx and reloads the tag list. Armed on call, so the response cannot arrive before the waiter (P02).
  const tags = page.waitForResponse("**/search.aspx");
  await page.locator("#author").selectOption("Albert Einstein");
  await tags;

  // App state, not element state (P01); the caller's own signal and a deadline compose natively.
  const results = page.waitForResponse("**/filter.aspx", { signal: AbortSignal.any([stop, AbortSignal.timeout(5_000)]) });
  await page.getByRole("button", { name: "Search" }).click();
  await results;
  const count = await page.locator(".results .quote").waitForCount({ min: 1 });

  try {
    await page.waitForLoadState("networkidle", { timeout: null }); // explicitly no timeout
  } catch (error) {
    if (error instanceof DeadlineExceededError) {
      // Names what it waited for, what it saw and where the limit came from, e.g. `timeout.wait`.
      log.warn(error.message, { limit: error.limit?.setting, observed: error.observed });
    }
    throw error;
  }

  // @ts-expect-error A fixed sleep is removed: the deprecation names the app-state waits instead (P37).
  await page.waitForTimeout(1_000);

  return count;
}
