// A handler's `enqueueLinks` with a Payload function on an Attached engine
// of Foxhound's executable of the same commit (ADRs 0093 and 0120; issue
// 1471): the function reads each element the locator matches, as
// `crawl_request.links` answers it, and gives each link its own Payload, or
// `null` to leave it out. The Rust twin of the engine's side is
// `tests/it/enqueue_links.rs` of Foxhound's crate, and Python's
// `test_link_payload_attached.py` of Foxhound's package is this test's twin.
import { Crawl, Foxhound, seed, type EnqueuedLinks, type MatchedLink } from "@goodfetch/foxhound";
import { provingGroundUrl, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { z } from "zod";

import { served } from "./served.ts";

/** The `links` family's host. */
const host = "links.test";

describe("enqueueLinks with a Payload function on an Attached engine", () => {
  test.override("product", "foxhound");
  test.override("provingGroundSeed", 863);

  test(
    "gives each link the Payload its function answers, and leaves out a link it answers null for",
    { tags: ["story:11-1021", "operation:crawl_request.enqueue_links"] },
    async ({ env, provingGround }) => {
      const url = (path: string): string => provingGroundUrl(provingGround, host, path);
      const crawl = new Crawl();
      const index = crawl.label("index");
      const page = crawl.label("page", {
        payload: z.object({ from: z.string().optional(), isbn: z.string().optional() }),
      });
      const read: MatchedLink[] = [];
      let answered: EnqueuedLinks | undefined;
      crawl.handle(index, async (context) => {
        answered = await context.enqueueLinks(context.page.locator("ul.links a"), page, (link) => {
          read.push(link);
          const isbn = link.attributes["data-isbn"];
          if (isbn !== undefined) return { isbn };
          return link.href === "2" ? null : { from: link.text };
        });
      });
      crawl.handle(page, () => {});
      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const foxhound = await Foxhound.connect({ env, settings: served(provingGround) });

      const run = await foxhound.startCrawl(crawl, { seeds: [seed(url("/answers/"), index)] });
      const report = await run.finish();
      await foxhound.close();

      expect(report.lifecycle).toBe("finished");
      expect(read.map(({ text }) => text)).toEqual([
        "One",
        "One again",
        "No link",
        "Mail",
        "Elsewhere",
        "Broken",
        "Two",
        "Three",
      ]);
      expect(answered?.links).toEqual([
        { kind: "added", id: "2", url: url("/answers/1") },
        { kind: "not_added", reason: "duplicate", url: "/answers/1#top" },
        { kind: "no_link" },
        { kind: "not_added", reason: "unsupported_scheme", url: "mailto:shop@links.test" },
        { kind: "not_added", reason: "out_of_scope", url: "http://elsewhere.test/" },
        { kind: "not_added", reason: "invalid_url", url: "http://[::1/" },
        { kind: "skipped_by_payload", url: "2" },
        { kind: "added", id: "3", url: url("/answers/3") },
      ]);
    },
  );
});
