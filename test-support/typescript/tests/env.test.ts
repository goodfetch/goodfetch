// Seam B (spec #325): the `env` fixture, the environment map an SDK takes
// (ADRs 0069, 0090 and 0094), from the real `goodfetch-test-env`. The
// workspace has no Product executable yet, so these tests hand it a tool
// manifest whose Product entries are stand-in paths.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, describe, expect } from "vitest";
import { test } from "@goodfetch/test-support";

const COLLIE = "/stand-in/goodfetch-collie";
const FOXHOUND = "/stand-in/goodfetch-foxhound";

/** The commit's manifest, with stand-in paths for both Products' executables. */
function standInManifest(): { folder: string; path: string } {
  const manifest = process.env["GOODFETCH_TEST_TOOLS"];
  if (manifest === undefined) {
    throw new Error("GOODFETCH_TEST_TOOLS is not set: run the tests with `just test`");
  }
  const folder = mkdtempSync(join(tmpdir(), "goodfetch-env-test-"));
  const path = join(folder, "test-tools.json");
  const tools: unknown = JSON.parse(readFileSync(manifest, "utf8"));
  writeFileSync(path, JSON.stringify({ ...Object(tools), collie: COLLIE, foxhound: FOXHOUND }));
  return { folder, path };
}

const manifest = standInManifest();
afterAll(() => {
  rmSync(manifest.folder, { recursive: true, force: true });
});

describe("env", () => {
  test.override("toolManifest", manifest.path);

  test("points Collie's engine-path variable at Collie's executable by default", ({
    env,
    testEnv,
  }) => {
    expect(env["GOODFETCH_COLLIE_ENGINE_PATH"]).toBe(COLLIE);
    expect(env).not.toHaveProperty("GOODFETCH_FOXHOUND_ENGINE_PATH");
    expect(env["GOODFETCH_HOME"]).toBe(testEnv.home);
  });
});

describe("env for a test that needs Foxhound", () => {
  test.override({ toolManifest: manifest.path, product: "foxhound" });

  test("points Foxhound's engine-path variable at Foxhound's executable", ({ env, testEnv }) => {
    expect(env["GOODFETCH_FOXHOUND_ENGINE_PATH"]).toBe(FOXHOUND);
    expect(env).not.toHaveProperty("GOODFETCH_COLLIE_ENGINE_PATH");
    expect(env["GOODFETCH_HOME"]).toBe(testEnv.home);
  });
});
