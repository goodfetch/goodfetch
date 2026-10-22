import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, test } from "vitest";
import { productEnv, scriptedEngineEnv } from "./product-env.ts";
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

describe("scriptedEngineEnv", () => {
  test("installs the script in the home it names and points the Product's variable at the Scripted engine", () => {
    const home = join(mkdtempSync(join(tmpdir(), "scripted-engine-env-")), "home");
    const description: TestEnvDescription = {
      ...DESCRIPTION,
      tools: { scripted_engine: "/bin/goodfetch-scripted-engine" },
    };

    const env = scriptedEngineEnv(description, "collie", "self-test/hello-as-collie.json", home);

    expect({
      env,
      script: readFileSync(join(home, "scripted-engine", "script.json"), "utf8"),
    }).toEqual({
      env: {
        GOODFETCH_HOME: home,
        TZ: "UTC",
        GOODFETCH_COLLIE_ENGINE_PATH: "/bin/goodfetch-scripted-engine",
      },
      script: readFileSync(
        new URL("../../fixtures/engine-scripts/self-test/hello-as-collie.json", import.meta.url),
        "utf8",
      ),
    });
  });
});
