import { describe, expect, test } from "vitest";
import { browserVerdict } from "./repeatability.ts";

const still = { file: "still.visual.spec.ts", title: "square" };

describe("browserVerdict", () => {
  test("repeats when every case ran the runs asked for and each repeated", () => {
    const verdict = browserVerdict("chromium", [{ ...still, runs: 20, repeated: 20 }], 20);

    expect(verdict.repeats).toBe(true);
  });

  test("does not repeat when a case ran fewer times than asked, though each run repeated", () => {
    const verdict = browserVerdict("chromium", [{ ...still, runs: 1, repeated: 1 }], 20);

    expect(verdict.repeats).toBe(false);
  });
});
