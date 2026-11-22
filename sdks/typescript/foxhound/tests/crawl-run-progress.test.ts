// A crawl's answer to `run.progress` (ADR 0102) reads into Foxhound's
// generated `CrawlRunProgress`: every Run's parts with the crawl's own,
// keys in camelCase and instants as dates, and a wait reason a newer
// engine sends kept as it came. Collie's `RunProgress` keeps the crawl's
// parts untyped; its twin is the Python package's
// `tests/unit/test_crawl_run_progress.py`.
import { fromWire } from "@goodfetch/collie-client/runtime";
import { describe, expect, expectTypeOf, test } from "vitest";
import type { CrawlRunProgress, WaitGroup } from "../src/generated/types.ts";
import { foxhoundWire } from "../src/generated/wire.ts";

describe("a crawl's progress", () => {
  test("reads into the crawl's progress type, a newer wait reason kept", () => {
    const sent = {
      run_id: "books",
      position: 4,
      at: "2026-01-05T09:00:00Z",
      lifecycle: "running",
      writer: "live",
      epoch: 1,
      pauses: [],
      last: false,
      waiting: [
        {
          reason: "domain_delay",
          count: 2,
          domains: [{ domain: "shop.test", next_admitted_at: "2026-01-05T09:00:01Z" }],
          setting: { key: "politeness.domain.delay", value: 1000, layer: "run" },
        },
        { reason: "a_reason_from_a_newer_engine", count: 1, domains: [{ domain: "a.test" }] },
      ],
      idle: false,
      concurrency: { handlers: 3, handler_concurrency: 8 },
    };

    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine answers are not validated beyond `hello`'s protocol version (STYLE.md): a crawl's `run.progress` answer is the catalog's `CrawlRunProgress`.
    const read = fromWire(
      sent,
      { type: "CrawlRunProgress" },
      foxhoundWire.types,
    ) as CrawlRunProgress;

    expectTypeOf(read.waiting).toEqualTypeOf<WaitGroup[] | null | undefined>();
    expect(read).toMatchObject({
      runId: "books",
      at: new Date("2026-01-05T09:00:00.000Z"),
      waiting: [
        {
          reason: "domain_delay",
          domains: [{ domain: "shop.test", nextAdmittedAt: new Date("2026-01-05T09:00:01.000Z") }],
        },
        { reason: "a_reason_from_a_newer_engine", count: 1 },
      ],
      idle: false,
      concurrency: { handlers: 3, handlerConcurrency: 8 },
    });
  });
});
