// Typed data (ADR 0082): a program types a value with a schema in its own
// types, Zod, ArkType or Valibot through Standard Schema and Standard JSON
// Schema, or a JSON Schema written by hand. `set` sends the schema's input
// form, `get` answers the output its parse builds; results keep the fields
// the SDK does not know; a value that is not JSON fails before it is sent,
// naming its path. The cases play on the Scripted engine, which checks every
// message the SDK sends. Supported ranges: Zod 4.2 and later, ArkType 2.1.28
// and later, Valibot 1.2 and later with `@valibot/to-json-schema` 1.5 and
// later (story 11-69); the dev dependencies pin each floor, so the cases run
// on the oldest version the SDK supports.
import {
  Collie,
  ValueInvalidError,
  type JSONObject,
  type JSONSchema,
  type Schema,
} from "@goodfetch/collie";
import { failure, recordingLogger, scriptedEngineEnv, test } from "@goodfetch/test-support";
import { toStandardJsonSchema } from "@valibot/to-json-schema";
import { type } from "arktype";
import * as v from "valibot";
import { describe, expect } from "vitest";
import { z } from "zod";

/** A book whose year the program reads as a number and stores as a string. */
const zodBook = z.object({
  title: z.string(),
  year: z.codec(z.string(), z.number(), { decode: Number, encode: String }),
});
const arktypeBook = type({ title: "string", year: "string.numeric.parse" });
const valibotBook = toStandardJsonSchema(
  v.object({ title: v.string(), year: v.pipe(v.string(), v.transform(Number)) }),
);
const handWrittenBook = {
  type: "object",
  properties: { title: { type: "string" }, year: { type: "string" } },
  required: ["title", "year"],
} as const satisfies JSONSchema;

describe("Typed data on the Scripted engine as the Attached engine", () => {
  test(
    "round-trips a value typed by Zod, ArkType, Valibot and a hand-written JSON Schema through the Key-value store",
    { tags: ["client_case/typed-value-round-trip", "attached", "story:11-66", "story:11-69"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/typed-value-round-trip.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const run = await collie.start({ runId: "typed" });
      const store = run.keyValueStore;

      await store.set("book-zod", { title: "Dune", year: "1965" }, zodBook);
      const fromZod = await store.get("book-zod", zodBook);
      await store.set("book-arktype", { title: "Dune", year: "1965" }, arktypeBook);
      const fromArktype = await store.get("book-arktype", arktypeBook);
      await store.set("book-valibot", { title: "Dune", year: "1965" }, valibotBook);
      const fromValibot = await store.get("book-valibot", valibotBook);
      await store.set("book-hand-written", { title: "Dune", year: "1965" }, handWrittenBook);
      const fromHandWritten = await store.get("book-hand-written", handWrittenBook);
      const unsent = await failure(
        store.set("book-zod", { title: "Dune", year: "nineteen" }, valibotRejecting()),
      );
      const legacy = await failure(store.get("book-legacy", zodBook));
      await run.finish();
      await collie.close();

      await clientCase.check(
        "each typed read answers the output its schema's parse builds, the year as a number",
        () => {
          expect(fromZod).toEqual({ title: "Dune", year: 1965 });
          expect(fromArktype).toEqual({ title: "Dune", year: 1965 });
          expect(fromValibot).toEqual({ title: "Dune", year: 1965 });
        },
      );
      await clientCase.check("a hand-written JSON Schema's read answers the JSON as stored", () =>
        expect(fromHandWritten).toEqual({ title: "Dune", year: "1965" }),
      );
      await clientCase.check(
        "a value the schema rejects fails with value_invalid naming its path, and is never sent",
        () => {
          expect(unsent).toBeInstanceOf(ValueInvalidError);
          expect(unsent).toMatchObject({
            code: "value_invalid",
            details: { key: "book-zod", refused: "value", reason: "value_schema", path: "/year" },
          });
        },
      );
      await clientCase.check(
        "a stored value the schema rejects fails the typed read with value_invalid naming its path",
        () =>
          expect(legacy).toMatchObject({
            code: "value_invalid",
            details: { key: "book-legacy", reason: "value_schema", path: "/year" },
          }),
      );
      await clientCase.check(
        "each set carried the input form and nothing else, so the script wrote no failure",
        () => expect(warned).toEqual([]),
      );
    },
  );

  test(
    "keeps a field the SDK does not know in the results it hands the program",
    { tags: ["client_case/result-keeps-unknown-fields", "attached", "story:11-68"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/result-keeps-unknown-fields.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });

      const run = await collie.start({ runId: "newer" });
      const report = await run.finish();
      await collie.close();

      await clientCase.check("the Run report keeps the newer engine's field as sent", () =>
        expect(report).toMatchObject({ runId: "newer", carbon: { grams: 1.5 } }),
      );
      await clientCase.check("the script played to its end, so its engine wrote no failure", () =>
        expect(warned).toEqual([]),
      );
    },
  );

  test(
    "refuses a value that is not JSON before sending it, naming its path, and sends `__proto__` keys as given",
    { tags: ["client_case/non-json-value-refused", "attached", "story:11-68"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/non-json-value-refused.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const run = await collie.start({ runId: "strict" });
      const store = run.keyValueStore;

      const notANumber = await failure(store.set("price", { amount: Number.NaN }));
      const map = await failure(
        // @ts-expect-error -- a `Map` is no JSON value (G3); this untyped call reaches the SDK's refusal.
        store.set("tags", { tags: [1, new Map([["a", 1]])] }),
      );
      // @ts-expect-error -- a `bigint` is no JSON value (G3); this untyped call reaches the SDK's refusal.
      const bigint = await failure(store.set("count", 10n));
      const keys: unknown = JSON.parse('{"__proto__": {"polluted": true}, "constructor": "kept"}');
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a parsed JSON object, whose `__proto__` key is an own property.
      await store.set("user-data", keys as JSONObject);
      await run.finish();
      await collie.close();

      await clientCase.check(
        "each fails with value_invalid, naming the key and the value's path",
        () => {
          for (const refused of [notANumber, map, bigint]) {
            expect(refused).toBeInstanceOf(ValueInvalidError);
          }
          expect(notANumber).toMatchObject({
            details: { key: "price", reason: "value_not_json", path: "/amount" },
          });
          expect(map).toMatchObject({
            details: { key: "tags", reason: "value_not_json", path: "/tags/1" },
          });
          expect(bigint).toMatchObject({
            details: { key: "count", reason: "value_not_json", path: "" },
          });
        },
      );
      await clientCase.check(
        "no refused value reached the engine, and `__proto__` crossed as an own key, so the script wrote no failure",
        () => expect(warned).toEqual([]),
      );
    },
  );
});

/** A Valibot schema that rejects a year that is not digits, at `/year`. */
function valibotRejecting(): Schema {
  return toStandardJsonSchema(
    v.object({ title: v.string(), year: v.pipe(v.string(), v.digits(), v.transform(Number)) }),
  );
}
