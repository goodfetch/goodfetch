// Seam B (spec #325): the `env` fixture, the environment map an SDK takes
// (ADRs 0069, 0090 and 0094), from the real `goodfetch-test-env`. The
// workspace has no Product executable yet, so these tests hand it a tool
// manifest whose Product entries are stand-in paths.
import { randomUUID } from "node:crypto";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect } from "vitest";
import { test } from "@goodfetch/test-support";

const COLLIE = "/stand-in/goodfetch-collie";
const FOXHOUND = "/stand-in/goodfetch-foxhound";

/** Where the commit's manifest goes, with stand-in paths for both Products' executables. */
const folder = join(tmpdir(), `goodfetch-env-test-${randomUUID()}`);
const manifest = { folder, path: join(folder, "test-tools.json") };

// Written before the tests run, not as the file loads: a listing of the
// tests (`vitest list`, which `xtask stress` runs) loads the file without
// `just test`'s variables.
beforeAll(() => {
  const commits = process.env["GOODFETCH_TEST_TOOLS"];
  if (commits === undefined) {
    throw new Error("GOODFETCH_TEST_TOOLS is not set: run the tests with `just test`");
  }
  mkdirSync(manifest.folder);
  const tools: unknown = JSON.parse(readFileSync(commits, "utf8"));
  writeFileSync(
    manifest.path,
    JSON.stringify({ ...Object(tools), collie: COLLIE, foxhound: FOXHOUND }),
  );
});
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
