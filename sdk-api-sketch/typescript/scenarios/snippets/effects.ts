// Snippet for Effects a program declares in code (#500, ADR 0106). No golden scenario places an
// order, so these programs are the sketch's own.
import { z } from "zod";
import { Crawl, EffectUncertainError, Foxhound, seed, type Logger, type Page, type Run } from "@goodfetch/foxhound";

declare const log: Logger;

// A Collie program safe to run again after a crash: the Run is named, so a start under the same ID
// resumes it, and the order placed before the crash comes back from the Effect journal.
export async function placeOrder(sku: string): Promise<string | null> {
  await using foxhound = await Foxhound.connect({ logger: log });
  await using run = await foxhound.start({ id: `order-${sku}` });
  const page = await run.newPage();
  await page.goto(`https://shop.example/products/${sku}`);
  await page.getByRole("button", { name: "Add to cart" }).click(); // not declared: adding twice is harmless here
  await page.goto("https://shop.example/checkout");

  const placed = await run.effect(`place-order-${sku}`, async () => {
    await page.getByRole("button", { name: "Place order" }).click();
    return (await page.getByTestId("order-number").textContent()).trim(); // kept in the journal
  });
  await run.finish();
  switch (placed.source) {
    case "performed":
    case "journal": // placed before a crash: the button was not pressed again
      return placed.output;
    case "reconciled": // a person said it happened; its number is not known
      log.warn("order placed, number unknown", { by: placed.reconciledBy });
      return null;
    case "unknown":
      return null;
  }
}

// A body that sends the site's own idempotency key is safe to repeat: declare it so.
export async function subscribe(run: Run, page: Page, plan: string): Promise<number> {
  const result = await run.effect(
    `subscribe-${plan}`,
    async (effect) => {
      const response = await page.request.post("https://shop.example/api/subscriptions", {
        headers: { "Idempotency-Key": effect.idempotencyKey },
        data: { plan },
      });
      return response.status();
    },
    { repeatable: true },
  );
  return result.source === "performed" || result.source === "journal" ? result.output : 0;
}

// In a crawl handler: one form submitted per request, never twice across retries, escalations,
// sample comparisons or a resume. The key's namespace is the crawl request.
const crawl = new Crawl();
const listing = crawl.label("listing", { payload: z.object({ email: z.email() }) });
crawl.handle(listing, async ({ page, request, run }) => {
  await page.getByLabel("Email").fill(request.payload.email); // preparing the page is no part of the Effect
  try {
    const sent = await run.effect("contact-seller", async () => {
      await page.getByRole("button", { name: "Contact seller" }).click();
      await page.getByText("Message sent").waitFor();
    });
    if (sent.source !== "performed") log.info("already contacted", { url: request.url, source: sent.source });
  } catch (error) {
    // The click went out and the confirmation never showed: a person reconciles it in the Console.
    if (error instanceof EffectUncertainError) log.warn("contact uncertain", { key: error.details.key, why: error.details.why });
    throw error; // the request ends `failed` with `effect_uncertain`, never retried
  }
});

export async function contactSellers(foxhound: Foxhound, emails: readonly string[]): Promise<void> {
  await using run = await foxhound.startCrawl(crawl, {
    id: "contact-2026-10-15",
    seeds: emails.map((email, i) => seed(`https://market.example/listings/${i}`, listing, { email })),
  });
  const report = await run.finish();
  for (const effect of report.uncertainEffects) log.warn("needs a person", { key: effect.key, request: effect.request?.url });
}
