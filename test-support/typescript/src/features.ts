// The Feature inventory as the Vitest and Playwright configurations read it
// (ADR 0072): `xtask codegen` writes `features/inventory.json`, validated
// here against the repository's committed `features/inventory.schema.json`.
// Vitest declares each Feature ID as a tag, so `strictTags` refuses a tag
// naming an unknown Feature; Playwright passes the IDs to the `covers`
// check of test support's `test`. The keyboard walk's served cases take
// each `screen:` Feature from it (`readScreens`, ADR 0073).
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { Ajv2020, type SchemaObject, type ValidateFunction } from "ajv/dist/2020.js";
import { TestSupportError } from "./errors.ts";
import type { Product } from "./product-env.ts";

/** The inventory's file, from the workspace's folder. */
const INVENTORY = join("features", "inventory.json");

/** What test support reads of the inventory; the schema checks the rest. */
interface Inventory {
  readonly features: readonly {
    readonly id: string;
    readonly kind: string;
    readonly status: "active" | "retired";
    readonly products: readonly Product[];
  }[];
}

const ajv = new Ajv2020({ allErrors: true });
// schemars writes Rust's `u32` as this format, which an area number is.
ajv.addFormat("uint32", {
  type: "number",
  validate: (value: number) => Number.isInteger(value) && value >= 0 && value <= 0xffff_ffff,
});

const schema: SchemaObject = JSON.parse(
  readFileSync(new URL("../../../features/inventory.schema.json", import.meta.url), "utf8"),
);
const isInventory: ValidateFunction<Inventory> = ajv.compile<Inventory>(schema);

/**
 * Every Feature ID of the inventory of the workspace at `root`, retired ones
 * included, so that a declaration of a retired Feature reaches the Feature
 * check; `inventory_invalid` when the file is missing or fails its schema.
 */
export function readFeatureIds(root: string): string[] {
  return readInventory(root).features.map((feature) => feature.id);
}

/** A Console screen, as its `screen:` Feature names it. */
export interface Screen {
  /** Its Feature's ID, `screen:<route>`, the route as the Console's router writes it (`screen:/runs/$run`). */
  readonly feature: string;
  /** The Product whose executable serves it in a test: Collie's for a screen both Products carry, Foxhound's for Foxhound's screens (ADR 0094). */
  readonly product: Product;
}

/** The active `screen:` Features of the inventory of the workspace at `root`; `inventory_invalid` as for `readFeatureIds`. */
export function readScreens(root: string): Screen[] {
  return readInventory(root)
    .features.filter((feature) => feature.kind === "screen" && feature.status === "active")
    .map((feature) => ({
      feature: feature.id,
      product: feature.products.includes("collie") ? "collie" : "foxhound",
    }));
}

function readInventory(root: string): Inventory {
  const path = join(root, INVENTORY);
  let inventory: unknown;
  try {
    inventory = JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    throw new TestSupportError(
      "inventory_invalid",
      `${path} cannot be read: run \`just codegen\`, which writes it`,
      { cause: error },
    );
  }
  if (!isInventory(inventory)) {
    throw new TestSupportError(
      "inventory_invalid",
      `${path} fails features/inventory.schema.json (${ajv.errorsText(isInventory.errors)}): run \`just codegen\`, which writes it`,
    );
  }
  return inventory;
}

/** A Playwright annotation, as `testInfo.annotations` holds it. */
export interface Annotation {
  readonly type: string;
  readonly description?: string | undefined;
}

/** The type of the annotation that declares a Feature in Playwright (ADR 0072). */
export const COVERS = "covers";

/**
 * Fails with `covers_unknown` when a `covers` annotation names no Feature of
 * the Playwright configuration's `metadata.features` (the inventory's IDs,
 * `readFeatureIds`), or names none at all. A configuration that passes no
 * list of IDs vouches for no annotation, so every `covers` annotation fails.
 */
export function refuseUnknownCovers(
  annotations: readonly Annotation[],
  metadata: Readonly<Record<string, unknown>>,
): void {
  const features = metadata["features"];
  const listed =
    Array.isArray(features) && features.every((feature) => typeof feature === "string");
  const known = new Set<unknown>(listed ? features : []);
  const unknown = annotations
    .filter((annotation) => annotation.type === COVERS)
    .map((annotation) => annotation.description ?? "")
    .filter((feature) => !known.has(feature));
  if (unknown.length > 0) {
    const where = listed
      ? "the inventory does not list"
      : "the Playwright configuration cannot check, since its `metadata.features` is no list of the inventory's IDs";
    throw new TestSupportError(
      "covers_unknown",
      `the test's covers annotations name Features ${where}: ${unknown.map((feature) => JSON.stringify(feature)).join(", ")}`,
    );
  }
}
