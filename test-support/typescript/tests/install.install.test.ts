// Seam B (spec #326): the install harness installs packed packages from a
// Verdaccio with no uplink, through each package manager, with its
// release-age gate on (ADR 0069). The registry serves two releases of the
// fixture Products: one published a month ago and the packed one, published
// now, which only ADR 0030's exclusion snippet lets through the gate.
//
// The Pre-merge suite takes one installed-package run, npm's; the Release
// suite runs every manager, and a manager its runner lacks fails the run
// with `manager_missing` (ADR 0069).
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect } from "vitest";
import { test as base } from "@goodfetch/test-support";
import {
  installPacked,
  PACKAGE_MANAGERS,
  startRegistry,
  type PackageManager,
  type Registry,
} from "@goodfetch/test-support/install";
import { COLLIE, FOXHOUND, SMOKE_SCRIPT, writeFixtureRelease } from "./install-products.ts";

const PREVIOUS = "0.1.0";
const PACKED = "0.2.0";

const MANAGERS: readonly PackageManager[] =
  process.env["GOODFETCH_TEST_SUITE"] === "release" ? PACKAGE_MANAGERS : ["npm"];

const test = base
  // oxlint-disable-next-line no-empty-pattern -- Vitest reads a fixture's dependencies from its first parameter, which must be a destructuring pattern.
  .extend("registry", { scope: "file" }, async ({}, { onCleanup }): Promise<Registry> => {
    const folder = mkdtempSync(join(tmpdir(), "goodfetch-registry-"));
    const registry = await startRegistry(join(folder, "registry"));
    onCleanup(async () => {
      await registry.close();
      rmSync(folder, { recursive: true, force: true });
    });
    const packages = join(folder, "packages");
    await registry.publish({ packages: writeFixtureRelease(packages, PREVIOUS), ageDays: 30 });
    await registry.publish({ packages: writeFixtureRelease(packages, PACKED) });
    return registry;
  })
  // oxlint-disable-next-line no-empty-pattern -- Vitest reads a fixture's dependencies from its first parameter, which must be a destructuring pattern.
  .extend("folder", async ({}, { onCleanup }): Promise<string> => {
    const folder = mkdtempSync(join(tmpdir(), "goodfetch-install-"));
    onCleanup(() => rmSync(folder, { recursive: true, force: true }));
    return folder;
  });

describe.each(MANAGERS)("installPacked with %s", (manager) => {
  test("installs both Products in one project, and both executables run", async ({
    registry,
    folder,
    processes,
  }) => {
    const project = await installPacked({
      manager,
      registry,
      folder,
      packages: [COLLIE.name, FOXHOUND.name],
      processes,
    });

    expect(project.folder).toMatch(/ café$/);
    expect(project.install).toMatchObject({ exit: { code: 0 } });
    const smoke = await project.runScript(SMOKE_SCRIPT, [PACKED, COLLIE.name, FOXHOUND.name]);
    expect(smoke).toMatchObject({
      exit: { code: 0 },
      stdout: "@goodfetch/collie, @goodfetch/foxhound 0.2.0\n",
    });
    const collie = await project.runExecutable(COLLIE.name, COLLIE.executable, []);
    expect(collie).toMatchObject({ exit: { code: 0 }, stdout: "goodfetch-collie 0.2.0\n" });
    const foxhound = await project.runExecutable(FOXHOUND.name, FOXHOUND.executable, []);
    expect(foxhound).toMatchObject({ exit: { code: 0 }, stdout: "goodfetch-foxhound 0.2.0\n" });
  });

  test.for([
    { product: COLLIE, printed: "goodfetch-collie 0.2.0\n" },
    { product: FOXHOUND, printed: "goodfetch-foxhound 0.2.0\n" },
  ])(
    "installs $product.name alone",
    async ({ product, printed }, { registry, folder, processes }) => {
      const project = await installPacked({
        manager,
        registry,
        folder,
        packages: [product.name],
        processes,
      });

      expect(project.install).toMatchObject({ exit: { code: 0 } });
      const smoke = await project.runScript(SMOKE_SCRIPT, [PACKED, product.name]);
      expect(smoke).toMatchObject({ exit: { code: 0 }, stdout: `${product.name} 0.2.0\n` });
      const run = await project.runExecutable(product.name, product.executable, []);
      expect(run).toMatchObject({ exit: { code: 0 }, stdout: printed });
    },
  );

  test("without the exclusion snippet, the gate holds the packed release back", async ({
    registry,
    folder,
    processes,
  }) => {
    const project = await installPacked({
      manager,
      registry,
      folder,
      packages: [COLLIE.name],
      exclusion: false,
      processes,
    });

    expect(project.install).toMatchObject({ exit: { code: 0 } });
    const run = await project.runExecutable(COLLIE.name, COLLIE.executable, []);
    expect(run).toMatchObject({ exit: { code: 0 }, stdout: "goodfetch-collie 0.1.0\n" });
  });
});

describe("installPacked with npm's --omit=optional", () => {
  test("fails at first use with Goodfetch's error", async ({ registry, folder, processes }) => {
    const project = await installPacked({
      manager: "npm",
      registry,
      folder,
      packages: [COLLIE.name],
      omitOptional: true,
      processes,
    });

    expect(project.install).toMatchObject({ exit: { code: 0 } });
    const run = await project.runExecutable(COLLIE.name, COLLIE.executable, ["--json"]);
    expect(run.exit.code).toBe(8);
    expect(JSON.parse(run.stdout)).toMatchObject({
      error: { code: "engine_not_found", group: "unavailable", retryable: false },
    });
  });
});
