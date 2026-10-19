import { describe, expect, test } from "vitest";
import { productEnv } from "./product-env.ts";
import type { TestEnvDescription } from "./test-env.ts";

const DESCRIPTION: TestEnvDescription = {
  folder: "/tmp/a test é x",
  home: "/tmp/a test é x/home",
  env: { GOODFETCH_HOME: "/tmp/a test é x/home", TZ: "UTC" },
  seed: 1,
  start_time: "2026-01-05T09:00:00Z",
  tools: { foxhound: "/bin/goodfetch-foxhound" },
};

describe("productEnv", () => {
  test("fails when the tool manifest lists no executable of the Product", () => {
    expect(() => productEnv(DESCRIPTION, "collie")).toThrow(
      expect.objectContaining({ code: "tool_missing" }),
    );
  });

  test("keeps the description's environment beside the engine-path variable", () => {
    const env = productEnv(DESCRIPTION, "foxhound");

    expect(env).toEqual({
      GOODFETCH_HOME: "/tmp/a test é x/home",
      TZ: "UTC",
      GOODFETCH_FOXHOUND_ENGINE_PATH: "/bin/goodfetch-foxhound",
    });
  });
});
