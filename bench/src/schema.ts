// The checks of what bench reads from outside it: its own data files,
// campaigns, the adapters' answers and the Proving ground's. Each is parsed
// against its JSON Schema where it crosses in, and trusted from then on.
import { Ajv2020, type SchemaObject, type ValidateFunction } from "ajv/dist/2020.js";
import { BenchError } from "./errors.ts";

const ajv = new Ajv2020({ allErrors: true });

/** A parser of one document kind, built from its schema. */
export interface Parser<T> {
  /** Returns `value` as a `T`, or throws `invalid_document` naming what it read and why. */
  parse(value: unknown, what: string): T;
}

/**
 * Compiles `schema` into a parser of `T`. The schema is the check: `T` must
 * describe exactly what it accepts.
 */
export function parserOf<T>(schema: SchemaObject): Parser<T> {
  const validate: ValidateFunction<T> = ajv.compile<T>(schema);
  return {
    parse(value, what) {
      if (validate(value)) {
        return value;
      }
      throw new BenchError("invalid_document", `${what}: ${ajv.errorsText(validate.errors)}`);
    },
  };
}
