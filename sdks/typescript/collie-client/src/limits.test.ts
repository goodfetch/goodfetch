// Limits in code (ADR 0080): the typed Limits a program gives, turned into
// the wire's list, a price's `Money` into exact millionths of its currency,
// and every amount that cannot be turned exactly refused before anything is
// sent.
import { describe, expect, test } from "vitest";
import { SettingInvalidError } from "./generated/index.ts";
import { limitList, type Limits, type Money } from "./limits.ts";

/** The `setting_invalid` that turning `limits` raises, or a failure when it turns. */
function refusal(limits: Limits): SettingInvalidError {
  try {
    limitList(limits);
  } catch (error) {
    if (error instanceof SettingInvalidError) {
      return error;
    }
    throw error;
  }
  throw new Error("the Limits turned");
}

describe("limitList", () => {
  test(
    "lists each count, then each price in millionths, by Usage kind",
    { tags: ["story:11-51"] },
    () => {
      const limits: Limits = {
        modelCalls: 100,
        browserTime: 60_000,
        pageCrashes: 0,
        price: {
          paidSolves: { amount: "2.5", currency: "USD" },
          proxyBytes: { amount: "0.000001", currency: "EUR" },
        },
      };

      expect(limitList(limits)).toEqual([
        { usageKind: "model_calls", amount: 100 },
        { usageKind: "browser_time", amount: 60_000 },
        { usageKind: "page_crashes", amount: 0 },
        { usageKind: "proxy_bytes", currency: "EUR", amount: 1 },
        { usageKind: "paid_solves", currency: "USD", amount: 2_500_000 },
      ]);
    },
  );

  test.each<[string, string, number]>([
    ["a whole amount", "3", 3_000_000],
    ["six decimals", "0.123456", 123_456],
    ["zeros past the sixth decimal", "1.1234560000", 1_123_456],
    ["the largest the wire carries exactly", "9007199254.740991", Number.MAX_SAFE_INTEGER],
  ])("turns %s exactly", { tags: ["story:11-51"] }, (_, amount, millionths) => {
    expect(limitList({ price: { modelTokens: { amount, currency: "USD" } } })).toEqual([
      { usageKind: "model_tokens", currency: "USD", amount: millionths },
    ]);
  });

  test("lists nothing for no Limits", { tags: ["story:11-51"] }, () => {
    expect([limitList(undefined), limitList({}), limitList({ price: {} })]).toEqual([[], [], []]);
  });

  test.each<[string, Money, string]>([
    ["a finer amount than a millionth", { amount: "0.0000001", currency: "USD" }, "millionth"],
    [
      "an amount past what the wire carries exactly",
      { amount: "9007199254.740992", currency: "USD" },
      "exactly",
    ],
    ["a malformed amount", { amount: "1,50", currency: "USD" }, "decimal"],
    ["a negative amount", { amount: "-1", currency: "USD" }, "decimal"],
    ["an exponent", { amount: "1e3", currency: "USD" }, "decimal"],
    ["a currency that is no ISO 4217 code", { amount: "1", currency: "usd" }, "ISO 4217"],
  ])("refuses %s as setting_invalid", { tags: ["story:11-51"] }, (_, money, reason) => {
    const refused = refusal({ price: { paidSolves: money } });

    expect(refused.details).toMatchObject({ key: "limits.price.paidSolves" });
    expect(String(refused.details["reason"])).toContain(reason);
  });

  test("refuses a Money whose amount is a number", { tags: ["story:11-51"] }, () => {
    // A JavaScript caller can pass a number where the type asks for a string.
    const money: Money = { amount: "0.5", currency: "USD" };
    Reflect.set(money, "amount", 0.5);

    const refused = refusal({ price: { paidSolves: money } });

    expect(refused.details).toMatchObject({ key: "limits.price.paidSolves" });
    expect(String(refused.details["reason"])).toContain("never a number");
  });

  test.each<[string, number]>([
    ["a fraction", 1.5],
    ["a negative count", -1],
    ["a count past what the wire carries exactly", Number.MAX_SAFE_INTEGER + 1],
    ["not a number", Number.NaN],
  ])("refuses %s as setting_invalid", { tags: ["story:11-51"] }, (_, count) => {
    expect(refusal({ requests: count }).details).toMatchObject({ key: "limits.requests" });
  });

  test("refuses a key that names no Usage kind", { tags: ["story:11-51"] }, () => {
    // A JavaScript caller can pass a key the type does not have.
    const limits: Limits = {};
    Reflect.set(limits, "steps", 3);

    expect(refusal(limits).details).toMatchObject({ key: "limits.steps" });
  });
});
