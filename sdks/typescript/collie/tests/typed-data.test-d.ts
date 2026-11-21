// Typed data's types (ADR 0082): what a program sends under a schema has its
// input type and what it receives its output type, for Zod, ArkType and
// Valibot through Standard Schema, and `JSONValue` under a hand-written JSON
// Schema; inputs are strict, so an unknown field fails to type-check, while
// results keep what the SDK does not know at run time (story 11-68).
import type {
  Collie,
  JSONSchema,
  JSONValue,
  KeyValueStore,
  SchemaInput,
  SchemaOutput,
} from "@goodfetch/collie";
import { toStandardJsonSchema } from "@valibot/to-json-schema";
import { type } from "arktype";
import * as v from "valibot";
import { expectTypeOf } from "vitest";
import { z } from "zod";

const zodBook = z.object({
  title: z.string(),
  year: z.codec(z.string(), z.number(), { decode: Number, encode: String }),
});
const arktypeBook = type({ title: "string", year: "string.numeric.parse" });
const valibotBook = toStandardJsonSchema(
  v.object({ title: v.string(), year: v.pipe(v.string(), v.transform(Number)) }),
);
const handWrittenBook = { type: "object" } as const satisfies JSONSchema;

/** The book as the program sends it, and as it reads it back. */
interface Sent {
  title: string;
  year: string;
}
interface Received {
  title: string;
  year: number;
}

expectTypeOf<SchemaInput<typeof zodBook>>().toEqualTypeOf<Sent>();
expectTypeOf<SchemaOutput<typeof zodBook>>().toEqualTypeOf<Received>();
expectTypeOf<SchemaInput<typeof arktypeBook>>().toEqualTypeOf<Sent>();
expectTypeOf<SchemaOutput<typeof arktypeBook>>().toEqualTypeOf<Received>();
expectTypeOf<SchemaInput<typeof valibotBook>>().toEqualTypeOf<Sent>();
expectTypeOf<SchemaOutput<typeof valibotBook>>().toEqualTypeOf<Received>();
expectTypeOf<SchemaInput<typeof handWrittenBook>>().toEqualTypeOf<JSONValue>();
expectTypeOf<SchemaOutput<typeof handWrittenBook>>().toEqualTypeOf<JSONValue>();

declare const store: KeyValueStore;
declare const collie: Collie;

expectTypeOf(store.get("book", zodBook)).resolves.toEqualTypeOf<Received | null>();
expectTypeOf(store.get("book", handWrittenBook)).resolves.toEqualTypeOf<JSONValue>();
expectTypeOf(store.get("book")).resolves.toEqualTypeOf<JSONValue>();

void store.set("book", { title: "Dune", year: "1965" }, zodBook);
// @ts-expect-error -- `set` takes the schema's input type: the year is sent as a string.
void store.set("book", { title: "Dune", year: 1965 }, zodBook);
// @ts-expect-error -- an input is strict: an unknown field fails to type-check.
void store.set("book", { title: "Dune", year: "1965", author: "Herbert" }, arktypeBook);
// @ts-expect-error -- `set` takes no top-level null (G3): `delete` removes a key.
void store.set("book", null, handWrittenBook);
// @ts-expect-error -- only JSON crosses (G3): a `Date` is no JSON value.
void store.set("book", { publishedOn: new Date() });
// @ts-expect-error -- an input is strict: `start` takes no unknown option.
void collie.start({ runId: "typed", retries: 3 });
