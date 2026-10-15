// Snippet for Collie's page API (#472, ADR 0085).
import {
  Collie,
  PageValueInvalidError,
  TargetNotUniqueError,
  type JSONValue,
  type Logger,
  type Page,
} from "@goodfetch/collie";

declare const log: Logger; // the program's own logger

export interface Quote {
  readonly text: string;
  readonly author: string;
  readonly tag: string;
}

// Golden scenario 01, "Fill a dependent form and read the results".
export async function einsteinQuotes(): Promise<Quote[]> {
  await using collie = await Collie.connect({ logger: log });
  await using run = await collie.start();
  await using page = await run.newPage();

  await page.goto("https://quotes.toscrape.com/search.aspx");
  // Choosing an author posts the form back; the action waits for that navigation to commit, and the
  // tag's option is waited for by the guard like any other condition (K01).
  await page.getByLabel("Author").selectOption("Albert Einstein");
  await page.getByLabel("Tag").selectOption("inspirational");
  const results = page.waitForResponse("**/filter.aspx"); // armed now, so the response cannot slip past (ADR 0080)
  await page.getByRole("button", { name: "Search" }).click();
  await results;

  // An empty list is an answer, not an error (G1). Reads run concurrently on the page, each in one
  // round trip; gestures would run one at a time (ADR 0085).
  const quotes = await page.locator(".results .quote").all();
  return Promise.all(
    quotes.map(async (quote) => ({
      text: await quote.locator(".content").textContent(),
      author: await quote.locator(".author").textContent(),
      tag: await quote.locator(".tag").textContent(),
    })),
  );
}

// Strictness: an ambiguous target is waited out like any other condition, then throws with every match.
export async function clickNext(page: Page): Promise<void> {
  try {
    await page.getByRole("link", { name: "Next" }).click({ timeout: 5_000 });
  } catch (error) {
    if (error instanceof TargetNotUniqueError) {
      // e.g. ["page.getByRole('link', { name: 'Next' }).nth(0)", …], rendered in this language.
      log.warn(error.message, { matches: error.details.matches });
      await page.getByRole("link", { name: "Next" }).first().click(); // the visible opt-out
      return;
    }
    throw error;
  }
}

// Routes and dialogs answer requests; one that answers nothing falls through (golden scenario 06's browser part).
export async function readScrollingQuotes(page: Page): Promise<JSONValue[]> {
  // Later routes run first; one that settles nothing falls through, then to the network.
  using _media = page.route("**/*", async (route) => {
    const type = route.request().resourceType();
    if (type === "image" || type === "font") return route.abort("blockedbyclient");
    return route.fallback();
  });
  // With no handler a confirm is dismissed anyway (a `beforeunload` accepted); this one accepts only an "Are you sure" confirm.
  using _dialogs = page.on("dialog", async (dialog) => {
    if (dialog.type() === "confirm" && dialog.message().startsWith("Are you sure")) await dialog.accept();
  });

  const pages: JSONValue[] = [];
  using _api = page.on("response", async (response) => {
    if (response.url().includes("/api/quotes") && response.ok()) pages.push(await response.json());
  });
  await page.goto("https://quotes.toscrape.com/scroll");
  await page.mouse.wheel(0, 10_000); // input with no target: marked unguarded
  const last = await page.waitForRequestFinished("**/api/quotes?page=*");
  const finalPage = await last.response(); // its body is kept: reading it never fails
  if (finalPage !== null) log.info("last page", { status: finalPage.status() });
  return pages;
}

// Page functions: JavaScript in the page, one explicit JSON argument, JSON back.
export async function readStats(page: Page): Promise<number> {
  const min = 3;
  // `min` is passed, not closed over: the page function sees only its argument.
  const long = await page.evaluate(
    ({ min }) => [...document.querySelectorAll(".tag")].filter((tag) => (tag.textContent ?? "").length > min).length,
    { min },
  );
  const next = await page.locator("li.next > a").getAttribute("href"); // `null` when absent: an answer
  const href = await page.locator("li.next > a").evaluate((link: Element) => link.getAttribute("href"));
  log.info("links", { next, href });
  try {
    // @ts-expect-error A DOM node is not JSON: the type checker refuses it before the engine would (G3).
    await page.evaluate(() => document.body);
  } catch (error) {
    if (error instanceof PageValueInvalidError) log.error(error.message, { path: error.details.path });
  }
  return long;
}

// Frames are reached through locators, same-origin or not; popups and downloads are values.
export async function payInFrame(page: Page): Promise<void> {
  await page.frameLocator("iframe[title='Checkout']").getByRole("button", { name: "Pay" }).click();
  await page.frameLocator().getByText("Paid").waitFor(); // every frame searched, one frame resolved

  const popup = page.waitForEvent("popup");
  await page.getByText("Open receipt").click();
  await using receipt = await popup; // a page of the same Session, closed with it
  const download = receipt.waitForEvent("download");
  await receipt.getByRole("button", { name: "PDF" }).click();
  await (await download).saveAs("receipt.pdf"); // streamed to this machine

  // Overlays that may appear at any time: handled before each attempt of a guarded action.
  using _newsletter = page.addLocatorHandler(page.getByText("Sign up to our newsletter"), async () => {
    await page.getByRole("button", { name: "No thanks" }).click();
  });
  await page.getByRole("link", { name: "Orders" }).click();
}

// A virtualized list renders only its visible rows, and no read scrolls (ADR 0121): the docs recipe
// scrolls the last rendered row into view and collects rows by key until none is new. On Chromium a
// row inside a closed shadow root is reached like any other.
export async function allRows(page: Page): Promise<Map<string, string>> {
  const rows = page.getByRole("table", { name: "Orders" }).getByRole("row");
  const all = new Map<string, string>();
  for (;;) {
    const seen = await rows.evaluateAll((els: Element[]) =>
      els.map((el): [string, string] => [el.getAttribute("aria-rowindex") ?? "", el.textContent ?? ""]),
    );
    const before = all.size;
    for (const [key, text] of seen) all.set(key, text);
    if (all.size === before) return all;
    await rows.last().scrollIntoViewIfNeeded(); // resolved again at each use: recycled nodes never go stale
  }
}

// The way down: raw CDP for one page, under ADR 0017's rules, Chromium only.
export async function heapSize(page: Page): Promise<JSONValue> {
  await using cdp = await page.newCDPSession();
  await cdp.send("Performance.enable");
  return cdp.send("Performance.getMetrics");
}

// @ts-expect-error Playwright's selector-taking page shortcuts are absent: use a locator (S2).
export const shortcut = (page: Page) => page.click("text=Next");
