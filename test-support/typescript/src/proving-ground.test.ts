import { describe, expect, test } from "vitest";
import { provingGroundOf } from "./proving-ground.ts";
import type { TestEnvDescription } from "./test-env.ts";

const DESCRIPTION: TestEnvDescription = {
  folder: "/tmp/a test é x",
  home: "/tmp/a test é x/home",
  env: { GOODFETCH_HOME: "/tmp/a test é x/home", TZ: "UTC" },
  seed: 1,
  start_time: "2026-01-05T09:00:00Z",
  tools: {},
};

describe("provingGroundOf", () => {
  test("fails when the description names no Proving ground", () => {
    expect(() => provingGroundOf(DESCRIPTION)).toThrow(
      expect.objectContaining({ code: "proving_ground_unseeded" }),
    );
  });
});
