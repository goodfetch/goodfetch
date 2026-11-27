// Module-level Crawl fixture imported by forked TypeScript worker processes
// in `tests/workers.test.ts` (spec #611's #1497; ADR 0024).
import { closeSync, openSync } from "node:fs";
import { setTimeout as delay } from "node:timers/promises";
import { Crawl, type Dataset, type Label } from "@goodfetch/foxhound";
import { z } from "zod";

export interface WorkerPayload {
  readonly starterPid?: number | undefined;
  readonly killMarker?: string | undefined;
  readonly delayMs?: number | undefined;
}

export interface HandledRecord {
  readonly url: string;
  readonly pid: number;
}

const workerPayloadSchema = z.object({
  starterPid: z.number().int().optional(),
  killMarker: z.string().optional(),
  delayMs: z.number().optional(),
});

const catalogCrawl: Crawl = new Crawl();
export const listingLabel: Label<WorkerPayload> = catalogCrawl.label("listing", {
  payload: workerPayloadSchema,
});
export const itemLabel: Label<WorkerPayload> = catalogCrawl.label("item", {
  payload: workerPayloadSchema,
});
export const pagesDataset: Dataset<HandledRecord, HandledRecord> =
  catalogCrawl.dataset<HandledRecord>("pages");

catalogCrawl.handle(listingLabel, async (ctx) => {
  const payload = workerPayloadSchema.parse(ctx.request.payload ?? {});
  await ctx.enqueueLinks(ctx.page.locator("li.product a"), itemLabel, payload);
  await ctx.enqueueLinks(ctx.page.locator("a[rel='next'], a.next"), listingLabel, payload);
});

catalogCrawl.handle(itemLabel, async (ctx) => {
  const payload = workerPayloadSchema.parse(ctx.request.payload ?? {});
  if (payload.delayMs !== undefined && payload.delayMs > 0) {
    await delay(payload.delayMs);
  }
  if (
    payload.killMarker !== undefined &&
    payload.starterPid !== undefined &&
    process.pid !== payload.starterPid
  ) {
    let shouldDie = false;
    try {
      const fd = openSync(payload.killMarker, "wx");
      closeSync(fd);
      shouldDie = true;
    } catch {
      // Another attempt already triggered the one-time worker kill.
    }
    if (shouldDie) {
      process.kill(process.pid, "SIGKILL");
    }
  }
  await ctx.push(pagesDataset, { url: ctx.request.url, pid: process.pid });
});

export default catalogCrawl;

export const namedCatalogCrawl: Crawl = catalogCrawl;

/** Function that creates a Crawl locally — NOT a module-level Crawl export. */
export function makeLocalCrawl(): Crawl {
  const local = new Crawl();
  const page = local.label("page");
  local.handle(page, async () => undefined);
  return local;
}
