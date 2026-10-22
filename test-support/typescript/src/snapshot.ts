// The shared snapshot redactions and filters (ADR 0062), read from
// `test-support/fixtures/snapshot-filters.json`, the one list insta and
// inline-snapshot apply too, so Vitest's snapshots redact as theirs do. A test
// snapshots `redact(output, values)`.
import { realpathSync } from "node:fs";
import type { ValidateFunction } from "ajv/dist/2020.js";
import { ajv, committedSchema, sharedFixture } from "./schemas.ts";

/** The values of one run that its snapshots redact beside the shared patterns. */
export interface SnapshotValues {
  /** The test's folder, `testEnv.folder`. */
  folder?: string | undefined;
  /** The Product version the test's executable reports. */
  productVersion?: string | undefined;
  /** The commit the test's executable reports. */
  commit?: string | undefined;
}

type RunValue = "folder" | "product_version" | "commit";

interface SnapshotFilters {
  values: { value: RunValue; replacement: string }[];
  patterns: { name: string; pattern: string; replacement: string }[];
}

const isSnapshotFilters: ValidateFunction<SnapshotFilters> = ajv.compile<SnapshotFilters>(
  committedSchema("snapshot-filters.schema.json"),
);

/** The field of `SnapshotValues` that holds each value the committed rules name. */
const VALUE_FIELDS: Record<RunValue, keyof SnapshotValues> = {
  folder: "folder",
  product_version: "productVersion",
  commit: "commit",
};

/**
 * The forms an output prints a folder in, longest first so a longer form is
 * replaced before a shorter one inside it: the path, its canonical path when
 * it has another, and each written inside a JSON string.
 */
function folderForms(folder: string): string[] {
  const paths = [folder];
  try {
    paths.push(realpathSync(folder));
  } catch {
    // A folder that does not exist has no canonical path.
  }
  const forms = new Set(paths.flatMap((path) => [path, JSON.stringify(path).slice(1, -1)]));
  return [...forms].toSorted(
    (left, right) => right.length - left.length || left.localeCompare(right),
  );
}

/**
 * `text` with the shared rules applied: `values` first, matched literally
 * (the folder in each form an output prints it in), then each committed
 * pattern, in the file's order. A word boundary is ASCII's, as in Rust and
 * Python.
 *
 * @throws {TestSupportError} `fixture_invalid` when the committed rules fail their schema.
 */
export function redact(text: string, values: SnapshotValues): string {
  const filters = sharedFixture("snapshot-filters.json", isSnapshotFilters);
  let redacted = text;
  for (const { value, replacement } of filters.values) {
    const literal = values[VALUE_FIELDS[value]];
    if (literal === undefined) continue;
    for (const form of value === "folder" ? folderForms(literal) : [literal]) {
      redacted = redacted.replaceAll(form, replacement);
    }
  }
  for (const { pattern, replacement } of filters.patterns) {
    redacted = redacted.replace(new RegExp(pattern, "g"), replacement);
  }
  return redacted;
}
