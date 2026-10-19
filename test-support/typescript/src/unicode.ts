// The shared Unicode record (ADR 0073), read from
// `test-support/fixtures/unicode-record.json`, the one file Rust and Python
// read too, so every Sink conformance case and Export format checks the same
// hard text round-trips byte for byte.
import type { ValidateFunction } from "ajv/dist/2020.js";
import { ajv, committedSchema, sharedFixture } from "./schemas.ts";

/** One field of the Unicode record. */
export interface UnicodeField {
  /** The field's name, in snake case. */
  name: string;
  /** Its text. */
  text: string;
}

/**
 * The Unicode record: one Record's fields, in the committed file's order:
 * `bidi`, `nfc`, `nfd`, `zwj_emoji`, `astral`, `zero_width_joiner`, `bom`
 * and `tab_and_newline`.
 */
export interface UnicodeRecord {
  fields: UnicodeField[];
}

const isUnicodeRecord: ValidateFunction<UnicodeRecord> = ajv.compile<UnicodeRecord>(
  committedSchema("unicode-record.schema.json"),
);

/** The committed Unicode record. */
export function unicodeRecord(): UnicodeRecord {
  return sharedFixture("unicode-record.json", isUnicodeRecord);
}
