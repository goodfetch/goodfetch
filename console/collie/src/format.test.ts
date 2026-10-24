// The Console's formatting (TESTING.md, Localization; ADR 0073): counts,
// amounts in millionths of a currency and points in time, through `Intl`
// with the locale and time zone given, over the shared table's locales and
// zones. Formatted text is asserted here only, on Node's pinned ICU.
import { locales, timeZones } from "@goodfetch/test-support";
import { describe, expect, test } from "vitest";
import { formatAmount, formatBytes, formatCount, formatInstant } from "./format.ts";

/** `text` with every kind of space as a plain one: ICU separates with no-break and narrow no-break spaces. */
function spaced(text: string): string {
  return text.replace(/\s/gu, " ");
}

describe("a count", () => {
  const expected: Readonly<Record<string, string>> = {
    "en-US": "1,234,567",
    "de-DE": "1.234.567",
    "fr-FR": "1 234 567",
    "tr-TR": "1.234.567",
    "ar-EG": "١٬٢٣٤٬٥٦٧",
    "hi-IN": "12,34,567",
    "ja-JP": "1,234,567",
  };
  test.each(locales())("is grouped as %s groups it", (locale) => {
    expect(spaced(formatCount(1_234_567, locale))).toBe(expected[locale]);
  });
});

describe("an amount", () => {
  test("of a count is the count", () => {
    expect(formatAmount(4_000, undefined, "en-US")).toBe("4,000");
  });

  test.each([
    { amount: 1_250_000, currency: "EUR", locale: "en-US", shown: "€1.25" },
    { amount: 1_250_000, currency: "EUR", locale: "de-DE", shown: "1,25 €" },
    { amount: 5_000_000, currency: "USD", locale: "fr-FR", shown: "5,00 $US" },
    { amount: 1_500_000_000, currency: "JPY", locale: "ja-JP", shown: "￥1,500" },
    { amount: 1, currency: "EUR", locale: "en-US", shown: "€0.000001" },
  ])(
    "of $amount millionths of $currency reads $shown in $locale",
    ({ amount, currency, locale, shown }) => {
      expect(spaced(formatAmount(amount, currency, locale))).toBe(shown);
    },
  );
});

describe("a size in bytes", () => {
  test.each([
    { bytes: 0, locale: "en-US", shown: "0 byte" },
    { bytes: 999, locale: "en-US", shown: "999 byte" },
    { bytes: 1_500, locale: "en-US", shown: "1.5 kB" },
    { bytes: 52_428_800, locale: "en-US", shown: "52.4 MB" },
    { bytes: 52_428_800, locale: "de-DE", shown: "52,4 MB" },
    { bytes: 3_000_000_000, locale: "en-US", shown: "3 GB" },
    { bytes: 2_500_000_000_000_000, locale: "en-US", shown: "2.5 PB" },
  ])("of $bytes reads $shown in $locale, in powers of 1,000", ({ bytes, locale, shown }) => {
    expect(spaced(formatBytes(bytes, locale))).toBe(shown);
  });
});

describe("a point in time", () => {
  // 2026-10-31T23:00:00Z, the wall clock of each shared zone, read off its offset that day.
  const expected: Readonly<Record<string, string>> = {
    UTC: "Oct 31, 2026, 11:00 PM",
    "America/St_Johns": "Oct 31, 2026, 8:30 PM",
    "Asia/Kathmandu": "Nov 1, 2026, 4:45 AM",
    "Pacific/Chatham": "Nov 1, 2026, 12:45 PM",
    "Australia/Lord_Howe": "Nov 1, 2026, 10:00 AM",
    "Europe/Paris": "Nov 1, 2026, 12:00 AM",
    "America/New_York": "Oct 31, 2026, 7:00 PM",
  };
  test.each(timeZones().map((zone) => zone.name))("is shown on the wall clock of %s", (zone) => {
    expect(spaced(formatInstant("2026-10-31T23:00:00Z", "en-US", zone))).toBe(expected[zone]);
  });

  test("is shown in the locale's own words", () => {
    expect(spaced(formatInstant("2026-10-31T23:00:00Z", "de-DE", "Europe/Paris"))).toBe(
      "01.11.2026, 00:00",
    );
  });
});
