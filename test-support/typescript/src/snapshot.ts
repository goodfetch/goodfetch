// The shared snapshot redactions and filters (ADR 0062), read from
// `test-support/fixtures/snapshot-filters.json`, the one list insta and
// inline-snapshot apply too, so Vitest's snapshots redact as theirs do. A test
// snapshots `redact(output, values)`.
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
 * `text` with the shared rules applied: `values` first, matched literally,
 * then each committed pattern, in the file's order.
 *
 * @throws {TestSupportError} `fixture_invalid` when the committed rules fail their schema.
 */
export function redact(text: string, values: SnapshotValues): string {
  const filters = sharedFixture("snapshot-filters.json", isSnapshotFilters);
  let redacted = text;
  for (const { value, replacement } of filters.values) {
    const literal = values[VALUE_FIELDS[value]];
    if (literal !== undefined) redacted = redacted.replaceAll(literal, replacement);
  }
  for (const { pattern, replacement } of filters.patterns) {
    redacted = redacted.replace(new RegExp(pattern, "g"), replacement);
  }
  return redacted;
}
