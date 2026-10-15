// Snippet for Decoding a response the same way on every tier (#550, ADR 0119).
import { z } from "zod";
import { Crawl, Foxhound, seed, type Logger } from "@goodfetch/foxhound";

declare const log: Logger;

const crawl = new Crawl();
const notices = crawl.dataset("notices", { schema: z.object({ title: z.string(), url: z.url() }) });
const files = crawl.dataset("files", { schema: z.object({ url: z.url(), contentType: z.string(), size: z.number().int() }) });
// HTML pages, the built-in `crawl.content_types`. The site's legacy section declares no charset, so its
// Label forces one rather than escalating each page to the browser.
const listing = crawl.label("listing", { settings: { fetch: { encoding: "shift_jis" } } });
// The Label takes PDFs too: their bytes are in `ctx.response`, and their page holds no document.
const attachment = crawl.label("attachment", {
  settings: { crawl: { contentTypes: ["text/html", "application/pdf"] } },
});

crawl.handle(listing, async ({ page, push, enqueueLinks }) => {
  await push(notices, { title: await page.locator("h1").textContent(), url: page.url() });
  await enqueueLinks(page.locator("a.attachment"), attachment);
});

crawl.handle(attachment, async ({ page, response, push, run }) => {
  const contentType = response.headers()["content-type"] ?? "";
  if (contentType.startsWith("application/pdf")) {
    // `page.locator(…)` here would throw `ResponseNotDocumentError` at once, on every tier.
    const bytes = await response.body();
    await run.keyValueStore.setBytes(`pdf/${encodeURIComponent(page.url())}`, bytes, "application/pdf");
    await push(files, { url: page.url(), contentType, size: bytes.length });
    return;
  }
  // `text()` decodes with the document's own encoding, so it agrees with `page.content()`.
  log.debug("attachment page", { url: page.url(), length: (await response.text()).length });
});

export async function decoding(): Promise<void> {
  await using foxhound = await Foxhound.connect({ logger: log });
  await using run = await foxhound.startCrawl(crawl, {
    seeds: [seed("https://example.org/notices/", listing)],
    settings: { export: { csv: { bom: true } } }, // the CSV Export opens in a spreadsheet
  });
  for await (const outcome of run.outcomes({ kinds: ["skipped"] })) {
    // A ZIP or an image under `attachment`: skipped before its body is read.
    if (outcome.kind === "skipped" && outcome.reason === "content_type") log.info("not collected", { url: outcome.request.url, type: outcome.message });
  }
  await run.finish();
  const csv = await run.export(notices, { format: "csv" }); // UTF-8, with a BOM under `export.csv.bom`
  await csv.saveAs("notices.csv");
}
