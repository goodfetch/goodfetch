// Seam B (spec #325): the `testEnv` fixture, used exactly as a product test
// uses it, against the real `goodfetch-test-env` the tool manifest names.
import { existsSync } from "node:fs";
import { join } from "node:path";
import { describe, expect } from "vitest";
import { test } from "@goodfetch/test-support";

describe("testEnv", () => {
  test("describes the test's folder, home and cleared environment", ({ testEnv }) => {
    expect(existsSync(testEnv.folder)).toBe(true);
    expect(testEnv.home).toBe(join(testEnv.folder, "home"));
    expect(testEnv.env["GOODFETCH_HOME"]).toBe(testEnv.home);
    expect(testEnv.env["TZ"]).toBe("UTC");
    expect(testEnv.start_time).toBe("2026-01-05T09:00:00Z");
  });
});
