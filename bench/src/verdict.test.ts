import { expect, test } from "vitest";
import { decide, type DecideInput, type Rule } from "./verdict.ts";

const superiority: Rule = { kind: "superiority", alpha: 0.05, power: 0.8 };
// Per-block ln(option / baseline) around `ratio`, with a small fixed jitter.
const blocks = (ratio: number, n: number, jitter = 0.01): number[] =>
  Array.from(
    { length: n },
    (_, index) => Math.log(ratio) + ((index % 2 === 1 ? jitter : -jitter) * ((index % 3) + 1)) / 3,
  );
const base: Omit<DecideInput, "logRatios"> = {
  direction: "lower",
  rule: superiority,
  delta: 0.1,
  gates: [{ id: "oracle_exact", ok: true }],
  seed: 11,
};

test("blocks the verdict on a failed contract gate, whatever the effect", () => {
  const verdict = decide({
    ...base,
    logRatios: blocks(0.5, 12),
    gates: [{ id: "oracle_exact", ok: false }],
  });

  expect(verdict.outcome).toBe("blocked-by-contract");
});

test("calls too few blocks, or too much spread for the margin, insufficient-n", () => {
  const noisy = decide({ ...base, logRatios: [0.5, -0.5, 0.5, -0.5, 0.5, -0.5, 0.5, -0.5] });

  expect(decide({ ...base, logRatios: blocks(0.5, 3) }).outcome).toBe("insufficient-n");
  expect(noisy.outcome).toBe("insufficient-n");
  expect(noisy.requiredN).toBeGreaterThan(8);
});

test("never asks fewer than five blocks, however tight the pilot", () => {
  const verdict = decide({ ...base, logRatios: blocks(0.5, 4, 0.000_1) });

  expect(verdict.outcome).toBe("insufficient-n");
  expect(verdict.requiredN).toBe(5);
});

test("calls a steady large cut of a lower-is-better metric better", () => {
  const verdict = decide({ ...base, logRatios: blocks(0.5, 12) });

  expect(verdict.outcome).toBe("better");
  expect(verdict.estimate?.ratio).toBeCloseTo(0.5, 1);
  expect(verdict.estimate?.hodgesLehmannRatio).toBeCloseTo(0.5, 1);
  expect(verdict.estimate?.wilcoxon.pValue).toBeLessThan(0.01);
});

test("calls a steady large rise of a lower-is-better metric worse", () => {
  expect(decide({ ...base, logRatios: blocks(1.6, 12) }).outcome).toBe("worse");
});

test("calls no effect inside the margin equivalent, and a small real one trivial", () => {
  expect(decide({ ...base, logRatios: blocks(1, 12, 0.01) }).outcome).toBe("equivalent");
  expect(decide({ ...base, logRatios: blocks(1.03, 12, 0.002) }).outcome).toBe("trivial");
});

test("accepts a slightly worse option inside the margin as non-inferior", () => {
  const verdict = decide({
    ...base,
    rule: { ...superiority, kind: "non-inferiority" },
    logRatios: blocks(1.02, 12, 0.003),
  });

  expect(verdict.outcome).toBe("non-inferior");
});

test("calls an interval across the margin inconclusive", () => {
  const verdict = decide({
    ...base,
    rule: { ...superiority, power: 0.5 },
    logRatios: [0.02, -0.18, 0.06, -0.14, 0.1, -0.1, 0.0, -0.16, 0.04, -0.12],
  });

  expect(verdict.outcome).toBe("inconclusive");
});
