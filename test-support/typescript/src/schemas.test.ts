import { expect, test } from "vitest";
import { ajv, committedSchema } from "./schemas.ts";

test("the observations' committed schema compiles, each format it names known to Ajv", () => {
  const schema = committedSchema("proving-ground-observations.schema.json");

  expect(() => ajv.compile(schema)).not.toThrow();
});

test("the format of Rust's `u16` takes an integer of 16 bits", () => {
  const isU16 = ajv.compile({ type: "number", format: "uint16" });

  expect([isU16(65_535), isU16(65_536), isU16(-1), isU16(1.5)]).toEqual([
    true,
    false,
    false,
    false,
  ]);
});
