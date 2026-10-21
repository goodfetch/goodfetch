// A Block cause as Collie's package types it (ADRs 0079 and 0095): the
// closed list's values Collie produces, and any other string an engine sends,
// such as Foxhound's `decoy` or a cause a later minor adds, kept as received.
import type { BlockCause } from "@goodfetch/collie";
import { test } from "@goodfetch/test-support";
import { describe, expect, expectTypeOf } from "vitest";

/** A Block cause as an engine's reply carries it, parsed and never checked. */
function received(wire: string): BlockCause {
  const parsed: { readonly block_cause: BlockCause } = JSON.parse(
    JSON.stringify({ block_cause: wire }),
  );
  return parsed.block_cause;
}

describe("a Block cause", () => {
  test("names each cause Collie produces", () => {
    expectTypeOf<
      "challenge" | "captcha" | "rate_limit" | "ip_block" | "payment_required" | "consent_required"
    >().toExtend<BlockCause>();
  });

  test("keeps a cause the package does not know as received", () => {
    const cause = received("tarpit");

    expectTypeOf<"decoy" | "tarpit">().toExtend<BlockCause>();
    expect(cause).toBe("tarpit");
  });
});
