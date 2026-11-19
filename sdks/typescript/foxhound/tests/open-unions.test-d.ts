// An open tagged union (ADR 0079): beside the members it knows, it has an
// `unknown` member keeping the tag a newer engine sent, and a program still
// narrows on each known member by its tag. `tests/open-values.test.ts` reads
// one; Python's `tests/unit/test_open_values.py` is its twin.
import type { StopReason, StoppedBy } from "@goodfetch/collie-client/generated";
import { expectTypeOf } from "vitest";
import type { AttemptError, CrawlRequestOutcome, SkipReason } from "../src/generated/types.ts";

declare const outcome: CrawlRequestOutcome;
declare const stop: StopReason;

switch (outcome.kind) {
  case "failed":
    expectTypeOf(outcome.error).toEqualTypeOf<AttemptError>();
    break;
  case "skipped":
    expectTypeOf(outcome.reason).toEqualTypeOf<SkipReason>();
    break;
  case "unknown":
    expectTypeOf(outcome.tag).toEqualTypeOf<string>();
    expectTypeOf(outcome.attempts).toEqualTypeOf<number>();
    break;
  default:
    break;
}

if (stop.kind === "abort") {
  expectTypeOf(stop.by).toEqualTypeOf<StoppedBy>();
} else if (stop.kind === "unknown") {
  expectTypeOf(stop.tag).toEqualTypeOf<string>();
}

// A member a newer engine adds is only ever read as `unknown`, never by its own tag.
// @ts-expect-error the tag a newer engine sent is in `tag`, not in `kind`
expectTypeOf<"a_newer_kind">().toExtend<StopReason["kind"]>();
