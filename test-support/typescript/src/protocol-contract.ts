// What the committed protocol contract allows on the wire: the JSON Schema
// bundle `xtask codegen` writes into `crates/goodfetch-protocol/generated/`
// (ADR 0038), compiled with Ajv, the one runtime validator of the
// TypeScript test tools (spec #326, ambiguity 6). The bundle is imported as
// a JSON module, so this module also runs in Vitest Browser Mode, where no
// file system is.
import { Ajv2020, type ValidateFunction } from "ajv/dist/2020.js";
import { fullFormats } from "ajv-formats/dist/formats.js";
import bundle from "../../../crates/goodfetch-protocol/generated/bundle.json" with { type: "json" };

/** Where the bundle's `$defs` are registered, so each catalog `$ref` resolves against them. */
const BUNDLE_ID = "https://goodfetch.dev/schemas/protocol-bundle.json";

const ajv = new Ajv2020({ allErrors: true });
// schemars marks each field with the protocol version it arrived in.
ajv.addKeyword("arrived_in");
ajv.addFormat("date-time", fullFormats["date-time"]);
for (const [format, minimum, maximum] of [
  ["uint8", 0, 2 ** 8 - 1],
  ["uint16", 0, 2 ** 16 - 1],
  ["int32", -(2 ** 31), 2 ** 31 - 1],
  ["uint32", 0, 2 ** 32 - 1],
  ["int64", Number.MIN_SAFE_INTEGER, Number.MAX_SAFE_INTEGER],
  ["uint64", 0, Number.MAX_SAFE_INTEGER],
] as const) {
  ajv.addFormat(format, {
    type: "number",
    validate: (value: number) =>
      Number.isSafeInteger(value) && value >= minimum && value <= maximum,
  });
}
ajv.addSchema({ $id: BUNDLE_ID, $defs: bundle.$defs });

/** A schema of the bundle or the catalog: a `$ref` into the bundle's `$defs`. */
interface Reference {
  readonly $ref: string;
}

/** What one schema of the contract finds wrong with a value: nothing, or Ajv's text. */
export type Problems = string | undefined;

/** A value checked against a schema: of its type, or broken, with Ajv's text. */
export type Checked<T> =
  | { readonly holds: true; readonly value: T }
  | { readonly holds: false; readonly problems: string };

/** Checks values against one schema of the contract, whose values are `T`s. */
export class Schema<T = unknown> {
  readonly #validate: ValidateFunction<T>;

  constructor(reference: Reference) {
    const validate = ajv.getSchema<T>(`${BUNDLE_ID}${reference.$ref}`);
    if (validate === undefined) {
      throw new Error(`invariant: the bundle defines ${reference.$ref}`);
    }
    this.#validate = validate;
  }

  /** `value` as a `T`, or what breaks this schema in it. */
  check(value: unknown): Checked<T> {
    return this.#validate(value)
      ? { holds: true, value }
      : { holds: false, problems: ajv.errorsText(this.#validate.errors) };
  }

  /** What breaks this schema in `value`, or `undefined` when it holds. */
  problems(value: unknown): Problems {
    const checked = this.check(value);
    return checked.holds ? undefined : checked.problems;
  }
}
