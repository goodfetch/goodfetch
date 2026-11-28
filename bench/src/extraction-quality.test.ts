import { describe, expect, test } from "vitest";
import { scoreExtractionQuality } from "./extraction-quality.ts";

describe("extraction quality scoring", () => {
  test("scores exact matches at 1.0 precision, recall, F1 and exact-match rate", () => {
    const expected = [
      { id: "p-1", name: "Widget", price: "19.99" },
      { id: "p-2", name: "Gadget", price: "29.50" },
    ];
    const report = scoreExtractionQuality({ actual: expected, expected });
    expect(report.exact_match_rate).toBe(1);
    expect(report.field_precision).toBe(1);
    expect(report.field_recall).toBe(1);
    expect(report.field_f1).toBe(1);
    expect(report.per_field.map((f) => f.field)).toEqual(["id", "name", "price"]);
  });

  test("penalizes wrong, missing and extra fields against ground truth", () => {
    const expected = [
      { id: "p-1", name: "Widget", price: "19.99" },
      { id: "p-2", name: "Gadget", price: "29.50" },
    ];
    const actual = [
      { id: "p-1", name: "Widget", price: "0.01" }, // wrong price
      { id: "p-2", name: "Gadget", price: "29.50", extra: "noise" }, // extra field
    ];
    const report = scoreExtractionQuality({ actual, expected });
    expect(report.records_exact_match).toBe(0);
    expect(report.field_recall).toBeCloseTo(5 / 6, 6);
    expect(report.field_precision).toBeCloseTo(5 / 7, 6);
    const priceField = report.per_field.find((f) => f.field === "price");
    expect(priceField?.matched).toBe(1);
    expect(priceField?.expected).toBe(2);
  });
});
