import { describe, expect, test } from "vitest";
import { locales, timeZones } from "./locales.ts";

describe("locales", () => {
  test("are ADR 0073's table", () => {
    const tags = locales();

    expect(tags).toEqual(["en-US", "de-DE", "fr-FR", "tr-TR", "ar-EG", "hi-IN", "ja-JP"]);
  });
});

describe("timeZones", () => {
  test("hold the DST days of Paris and New York", () => {
    const zones = timeZones();

    expect(zones).toEqual([
      { name: "UTC" },
      { name: "America/St_Johns" },
      { name: "Asia/Kathmandu" },
      { name: "Pacific/Chatham" },
      { name: "Australia/Lord_Howe" },
      { name: "Europe/Paris", dst_gap: "2026-03-29", dst_overlap: "2026-10-25" },
      { name: "America/New_York", dst_gap: "2026-03-08", dst_overlap: "2026-11-01" },
    ]);
  });
});
