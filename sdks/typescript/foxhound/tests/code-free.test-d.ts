// A code-free crawl's start (ADRs 0093 and 0102, G3): only a code-free
// crawl takes `detached`, since a program's crawl waits on its handlers,
// and a start names a declaration or an extraction, never both. Python's
// `tests/types/code_free_types.py` is its twin.
import { expectTypeOf } from "vitest";
import type { Crawl, CrawlDeclaration, CrawlRun, Foxhound, Seed } from "../src/index.ts";

declare const foxhound: Foxhound;
declare const crawl: Crawl;
declare const declaration: CrawlDeclaration;
const seeds: [Seed] = [{ url: "https://shop.test/", label: "page" }];

expectTypeOf(
  foxhound.startCrawl({ extraction: { formats: { markdown: true } } }, { seeds, detached: true }),
).resolves.toEqualTypeOf<CrawlRun>();
expectTypeOf(
  foxhound.startCrawl(declaration, { seeds, detached: true }),
).resolves.toEqualTypeOf<CrawlRun>();

// @ts-expect-error a program's crawl waits on its handlers, so it never detaches
void foxhound.startCrawl(crawl, { seeds, detached: true });
// @ts-expect-error a start names a declaration or an extraction, not both
void foxhound.startCrawl({ ...declaration, extraction: { formats: {} } }, { seeds });
