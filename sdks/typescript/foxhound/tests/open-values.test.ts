// A newer engine's values (ADR 0079): an outcome kind, skip reason, enqueue
// refusal's reason, Block cause, Hook stage, Verdict, Usage kind or Limit
// scope this package does not know reads as the string the engine sent,
// typed by its open enumeration,
// a member of an open tagged union it does not know reads as its `unknown`
// member with the tag it received, and an unknown Verdict is never
// `verified`. Foxhound's values of a type both Products share reach a Collie
// program the same way (ADR 0095).
import type {
  LimitScope as CollieLimitScope,
  UsageKind as CollieUsageKind,
  Verdict,
} from "@goodfetch/collie-client/generated";
import { fromWire } from "@goodfetch/collie-client/runtime";
import { describe, expect, expectTypeOf, test } from "vitest";
import type {
  BlockCause,
  CrawlHookStage,
  NotAddedReason,
  OutcomeKind,
  SkipReason,
} from "../src/generated/types.ts";
import { foxhoundWire } from "../src/generated/wire.ts";

describe("a value a newer engine adds", () => {
  test("is kept as received in an outcome, never refused", () => {
    const sent = {
      outcomes: [
        {
          kind: "skipped",
          reason: "a_newer_reason",
          request: { id: "r1", url: "https://shop.test/", label: "page" },
          ended_at: "2026-11-19T08:00:00Z",
        },
        {
          kind: "blocked",
          cause: "a_newer_cause",
          request: { id: "r2", url: "https://shop.test/2", label: "page" },
          ended_at: "2026-11-19T08:00:00Z",
        },
      ],
    };

    const read = fromWire(sent, foxhoundWire.results["run.outcomes"], foxhoundWire.types);

    expect(read).toMatchObject({
      outcomes: [
        { kind: "skipped", reason: "a_newer_reason" },
        { kind: "blocked", cause: "a_newer_cause" },
      ],
    });
  });

  test("in an outcome reads as its `unknown` member, the fields every outcome shares read and the rest as sent", () => {
    const sent = {
      outcomes: [
        {
          kind: "a_newer_kind",
          attempts: 1,
          request: { id: "r1", url: "https://shop.test/", label: "page" },
          ended_at: "2026-11-19T08:00:00Z",
          its_own_field: "kept",
        },
        {
          kind: "skipped",
          reason: "duplicate",
          request: { id: "r2", url: "https://shop.test/2", label: "page" },
          ended_at: "2026-11-19T08:00:00Z",
        },
      ],
    };

    const read = fromWire(sent, foxhoundWire.results["run.outcomes"], foxhoundWire.types);

    expect(read).toStrictEqual({
      outcomes: [
        {
          kind: "unknown",
          tag: "a_newer_kind",
          attempts: 1,
          request: { id: "r1", url: "https://shop.test/", label: "page" },
          endedAt: new Date("2026-11-19T08:00:00.000Z"),
          its_own_field: "kept",
        },
        {
          kind: "skipped",
          reason: "duplicate",
          request: { id: "r2", url: "https://shop.test/2", label: "page" },
          endedAt: new Date("2026-11-19T08:00:00.000Z"),
        },
      ],
    });
  });

  test("in a Collie union a Foxhound result holds reads as its `unknown` member", () => {
    const report = {
      run_id: "run-1",
      stop_reason: { kind: "a_newer_reason", by_whom: "x", resets_at: "2026-11-19T08:00:00Z" },
    };

    const read = fromWire(report, { type: "CrawlRunReport" }, foxhoundWire.types);

    expect(read).toStrictEqual({
      runId: "run-1",
      stopReason: {
        kind: "unknown",
        tag: "a_newer_reason",
        by_whom: "x",
        resets_at: "2026-11-19T08:00:00Z",
      },
    });
  });

  test("types each open enumeration as its values or any other string", () => {
    expectTypeOf<"a_newer_value">().toExtend<OutcomeKind>();
    expectTypeOf<"a_newer_value">().toExtend<SkipReason>();
    expectTypeOf<"a_newer_value">().toExtend<NotAddedReason>();
    expectTypeOf<"a_newer_value">().toExtend<BlockCause>();
    expectTypeOf<"a_newer_value">().toExtend<CrawlHookStage>();
    expectTypeOf<"a_newer_value">().toExtend<Verdict>();
    expectTypeOf<"a_newer_value">().toExtend<CollieUsageKind>();
    expectTypeOf<"a_newer_value">().toExtend<CollieLimitScope["kind"]>();
  });

  test("never reads as `verified` when it is an unknown Verdict", () => {
    const verdict: Verdict = "a_newer_verdict";

    expect(verdict === "verified").toBe(false);
  });

  test("of Foxhound's own reaches a Collie program as a string", () => {
    const usage: CollieUsageKind = "paid_fetches";
    const scope: CollieLimitScope = { kind: "partitioned_crawl", id: "books" };

    expect([usage, scope.kind]).toEqual(["paid_fetches", "partitioned_crawl"]);
  });
});
