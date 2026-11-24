// The committed JSON Schemas of test support's file types (ADR 0062), which
// `xtask codegen` writes from `goodfetch-test-support`'s Rust types into
// `test-support/fixtures/schemas/`, so Rust, TypeScript and Python read each
// file by the same contract; and the shared fixtures beside them.
import { readFileSync } from "node:fs";
import { Ajv2020, type SchemaObject, type ValidateFunction } from "ajv/dist/2020.js";
import { fullFormats } from "ajv-formats/dist/formats.js";
import { TestSupportError } from "./errors.ts";

/** The one Ajv instance test support compiles its schemas with. */
export const ajv: Ajv2020 = new Ajv2020({ allErrors: true });
ajv.addFormat("date", fullFormats.date);
ajv.addFormat("date-time", fullFormats["date-time"]);
// schemars writes Rust's `u64` as this format; the description's seed is also
// at most 2^53 - 1, so every language reads it exactly.
ajv.addFormat("uint64", {
  type: "number",
  validate: (value: number) => Number.isSafeInteger(value) && value >= 0,
});
// And Rust's `i64` as this one, such as a webhook's Unix timestamp, read
// exactly up to 2^53 - 1 either way.
ajv.addFormat("int64", {
  type: "number",
  validate: (value: number) => Number.isSafeInteger(value),
});
// schemars writes Rust's `u8` as this one, such as a script's exit code.
ajv.addFormat("uint8", {
  type: "number",
  validate: (value: number) => Number.isInteger(value) && value >= 0 && value <= 255,
});
// And Rust's `u16`, such as a status in the Proving ground's observations.
ajv.addFormat("uint16", {
  type: "number",
  validate: (value: number) => Number.isInteger(value) && value >= 0 && value <= 65_535,
});

/** A committed schema of `test-support/fixtures/schemas/`, which Ajv checks as it compiles it. */
export function committedSchema(file: string): SchemaObject {
  const committed: SchemaObject = JSON.parse(
    readFileSync(new URL(`../../fixtures/schemas/${file}`, import.meta.url), "utf8"),
  );
  return committed;
}

/**
 * Reads the shared fixture `file` of `test-support/fixtures/`, checked by
 * `isValid`, compiled from its committed schema.
 *
 * @throws {TestSupportError} `fixture_invalid` when the file fails its schema.
 */
export function sharedFixture<T>(file: string, isValid: ValidateFunction<T>): T {
  const value: unknown = JSON.parse(
    readFileSync(new URL(`../../fixtures/${file}`, import.meta.url), "utf8"),
  );
  if (!isValid(value)) {
    throw new TestSupportError(
      "fixture_invalid",
      `test-support/fixtures/${file} fails its committed schema: ${ajv.errorsText(isValid.errors)}`,
    );
  }
  return value;
}
