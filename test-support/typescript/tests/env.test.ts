// Seam B (spec #325): the `env` fixture, the environment map an SDK takes
// (ADRs 0069, 0090 and 0094), from the real `goodfetch-test-env` and the
// commit's executables, which the test tool manifest names.
import { readFileSync } from "node:fs";
import { describe, expect } from "vitest";
import { test } from "@goodfetch/test-support";

/** The executable of `product` the commit's test tool manifest names. */
function executable(product: "collie" | "foxhound"): string {
  const manifest = process.env["GOODFETCH_TEST_TOOLS"];
  if (manifest === undefined) {
    throw new Error("GOODFETCH_TEST_TOOLS is not set: run the tests with `just test`");
  }
  const tools: Record<string, string> = JSON.parse(readFileSync(manifest, "utf8"));
  return tools[product] ?? "";
}

describe("env", () => {
  test("points Collie's engine-path variable at Collie's executable by default", ({
    env,
    testEnv,
  }) => {
    expect(env["GOODFETCH_COLLIE_ENGINE_PATH"]).toBe(executable("collie"));
    expect(env).not.toHaveProperty("GOODFETCH_FOXHOUND_ENGINE_PATH");
    expect(env["GOODFETCH_HOME"]).toBe(testEnv.home);
  });
});

describe("env for a test that needs Foxhound", () => {
  test.override("product", "foxhound");

  test("points Foxhound's engine-path variable at Foxhound's executable", ({ env, testEnv }) => {
    expect(env["GOODFETCH_FOXHOUND_ENGINE_PATH"]).toBe(executable("foxhound"));
    expect(env).not.toHaveProperty("GOODFETCH_COLLIE_ENGINE_PATH");
    expect(env["GOODFETCH_HOME"]).toBe(testEnv.home);
  });
});
