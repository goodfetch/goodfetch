// A Block cause as Collie's package types it (ADRs 0079 and 0095): the
// closed list's values Collie produces, and any other string an engine sends,
// such as Foxhound's `decoy` or a cause a later minor adds, kept as received.
import type { BlockCause } from "@goodfetch/collie";
import { test } from "@goodfetch/test-support";
import { describe, expectTypeOf } from "vitest";

/** The values a generated open type names: its members but the bare string. */
type Named<Open> = Open extends string ? (string extends Open ? never : Open) : never;

describe("a Block cause", () => {
  test("names each cause Collie produces", () => {
    expectTypeOf<Named<BlockCause>>().toEqualTypeOf<
      "challenge" | "captcha" | "rate_limit" | "ip_block" | "payment_required" | "consent_required"
    >();
  });

  test("keeps a cause the package does not know as received", () => {
    expectTypeOf<"decoy" | "tarpit">().toExtend<BlockCause>();
  });
});
