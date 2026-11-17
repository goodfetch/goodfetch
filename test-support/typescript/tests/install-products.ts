// The two fixture Products the install harness's tests publish (spec #326,
// Seam B): packages named as Collie's and Foxhound's are, so ADR 0030's
// exclusion snippets cover them, each with the six platform packages it
// depends on optionally, and the Collie client package both pin exactly
// (ADRs 0094 and 0095).
import { copyFileSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const templates = new URL("fixtures/install/", import.meta.url);

/** A fixture Product: its package and its executable. */
export interface FixtureProduct {
  readonly name: string;
  readonly executable: string;
}

export const COLLIE: FixtureProduct = { name: "@goodfetch/collie", executable: "goodfetch-collie" };
export const FOXHOUND: FixtureProduct = {
  name: "@goodfetch/foxhound",
  executable: "goodfetch-foxhound",
};
const CLIENT = "@goodfetch/collie-client";

/** The six platforms each Product ships a platform package for (ADR 0094). */
const PLATFORMS = [
  ["linux", "x64"],
  ["linux", "arm64"],
  ["darwin", "x64"],
  ["darwin", "arm64"],
  ["win32", "x64"],
  ["win32", "arm64"],
] as const;

/** The smoke script the fixture Products' tests run in an installed project. */
export const SMOKE_SCRIPT: string = fileURLToPath(new URL("smoke.mjs", templates));

/**
 * Writes one release of both fixture Products at `version` under `folder`,
 * and gives the folders of its packages: the client first, then each
 * Product's platform packages and the Product.
 */
export function writeFixtureRelease(folder: string, version: string): readonly string[] {
  const folders = [writePackage(folder, CLIENT, version, { main: "client.mjs" })];
  for (const product of [COLLIE, FOXHOUND]) {
    const platformPackages = PLATFORMS.map(([os, cpu]) => {
      const name = `${product.name}-${os}-${cpu}`;
      folders.push(
        writePackage(folder, name, version, { main: "engine.mjs", os: [os], cpu: [cpu] }),
      );
      return name;
    });
    folders.push(
      writePackage(folder, product.name, version, {
        main: "index.mjs",
        bin: { [product.executable]: "bin.mjs" },
        dependencies: { [CLIENT]: version },
        optionalDependencies: Object.fromEntries(platformPackages.map((name) => [name, version])),
      }),
    );
  }
  return folders;
}

interface PackageShape {
  readonly main: "client.mjs" | "engine.mjs" | "index.mjs";
  readonly bin?: Readonly<Record<string, string>> | undefined;
  readonly os?: readonly string[] | undefined;
  readonly cpu?: readonly string[] | undefined;
  readonly dependencies?: Readonly<Record<string, string>> | undefined;
  readonly optionalDependencies?: Readonly<Record<string, string>> | undefined;
}

function writePackage(folder: string, name: string, version: string, shape: PackageShape): string {
  const packageFolder = join(folder, version, name);
  mkdirSync(packageFolder, { recursive: true });
  const files = [shape.main, ...(shape.bin === undefined ? [] : ["bin.mjs"])];
  for (const file of files) {
    copyFileSync(new URL(file, templates), join(packageFolder, file));
  }
  const manifest = {
    name,
    version,
    type: "module",
    exports: {
      ".": `./${shape.main}`,
      [`./${shape.main}`]: `./${shape.main}`,
      "./package.json": "./package.json",
    },
    files,
    ...shape,
  };
  writeFileSync(join(packageFolder, "package.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  return packageFolder;
}
