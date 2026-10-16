// Snippet for A disk that fills under a long Run, and log volume (#553, ADR 0122), on scenario 07's crawl.
import { Foxhound, ResourcesExhaustedError, seed, type Crawl, type CrawlRun, type CrawlRunReport, type Label, type Logger } from "@goodfetch/foxhound";

declare const log: Logger;
declare const books: Crawl;
declare const home: Label<undefined>;

// A long crawl on a small disk: below `storage.critical_free` it stops resumably, never half-written,
// and this same program run again under its ID carries on once space is freed.
export async function crawlOnASmallDisk(): Promise<CrawlRunReport | null> {
  await using foxhound = await Foxhound.connect({ logger: log });
  try {
    await using run = await foxhound.startCrawl(books, {
      runId: "books-2026-10-15",
      seeds: [seed("https://books.toscrape.com/", home)],
    });
    const report = await run.finish();
    const reason = report.stopReason;
    if (reason?.kind === "resources" && reason.resource === "storage") {
      log.warn("stopped before the disk filled: free space, then run again", { path: reason.path });
    }
    for (const warning of report.warnings) if (warning.code === "storage_low") log.warn(warning.message);
    return report;
  } catch (error) {
    // Only where no report says it: a start refused on a full filesystem, or a stop it could not record.
    if (error instanceof ResourcesExhaustedError) {
      log.error("storage full", { path: error.details.path, freeBytes: error.details.freeBytes });
      return null;
    }
    throw error;
  }
}

/** While it runs: the worst storage level among the filesystems it writes to. */
export async function followStorage(run: CrawlRun): Promise<void> {
  for await (const now of run.progress()) {
    if (now.storage.level !== "normal") log.warn("storage", { level: now.storage.level, path: now.storage.path, freeBytes: now.storage.freeBytes });
  }
}
