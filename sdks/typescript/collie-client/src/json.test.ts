// Only JSON values cross (ADR 0082, G3): a value a program sends is
// checked before anything is sent, and the first value that is not JSON is
// named by its JSON Pointer. User data keeps its keys, `__proto__` included,
// and comes back unchanged (TESTING.md, SDKs: mapping and decoding are
// property-tested).
import * as fc from "fast-check";
import { describe, expect, test } from "vitest";
import { encodeMessage, parseLine } from "./framing.ts";
import { nonJSON, type JSONObject, type JSONValue } from "./json.ts";
import { KeyValueStore, type KeyValueStoreHost, type StoredValue } from "./key-value-store.ts";

/** Any JSON value, whose object keys include `__proto__` and pointer escapes. */
const jsonValue: fc.Arbitrary<JSONValue> = fc.letrec<{ value: JSONValue }>((tie) => ({
  value: fc.oneof(
    { depthSize: "small" },
    fc.constant(null),
    fc.boolean(),
    fc.double({ noNaN: true, noDefaultInfinity: true }),
    fc.string(),
    fc.array(tie("value"), { maxLength: 4 }),
    fc.dictionary(
      fc.oneof(fc.constantFrom("__proto__", "constructor", "a/b", "~", ""), fc.string()),
      tie("value"),
      { maxKeys: 4 },
    ),
  ),
})).value;

/** A value that is not JSON, with how the check names it. */
const notJSON = fc.constantFrom<[unknown, string]>(
  [Number.NaN, "NaN"],
  [Number.POSITIVE_INFINITY, "Infinity"],
  [Number.NEGATIVE_INFINITY, "-Infinity"],
  [10n, "a bigint"],
  [new Map([["a", 1]]), "a Map"],
  [new Set([1]), "a Set"],
  [new Date(0), "a Date"],
  [new Uint8Array([1]), "a Uint8Array"],
  [() => 1, "a function"],
  [Symbol("s"), "a symbol"],
);

/** Whether `value` is a JSON list, which `Array.isArray` does not narrow to when readonly. */
function isList(value: readonly JSONValue[] | JSONObject): value is readonly JSONValue[] {
  return Array.isArray(value);
}

/** `root` with `leaf` set at the place `steps` picks, and that place's JSON Pointer. */
function planted(root: JSONValue, leaf: unknown, steps: number[]): [unknown, string] {
  if (steps.length === 0 || root === null || typeof root !== "object") {
    return [leaf, ""];
  }
  const [step = 0, ...rest] = steps;
  if (isList(root)) {
    const items: unknown[] = [...root];
    const index = items.length === 0 ? 0 : step % items.length;
    const [child, pointer] = planted(root[index] ?? null, leaf, rest);
    items[index] = child;
    return [items, `/${String(index)}${pointer}`];
  }
  const keys = Object.keys(root);
  const key = keys.length === 0 ? "k" : (keys[step % keys.length] ?? "k");
  const [child, pointer] = planted(root[key] ?? null, leaf, rest);
  // `Object.fromEntries` defines each key as its own, `__proto__` included.
  const copy = Object.fromEntries([...Object.entries(root), [key, child]]);
  return [copy, `/${key.replaceAll("~", "~0").replaceAll("/", "~1")}${pointer}`];
}

/** A host method this file's tests never call. */
function unused(): never {
  throw new Error("not used by this test");
}

/**
 * A store host that keeps each value as the line the SDK frames for the
 * engine, and answers it as a line from the engine is read back: what a
 * value crosses on the wire, both ways.
 */
function framingHost(): KeyValueStoreHost {
  const lines = new Map<string, string>();
  return {
    get: async (_runId, key) => {
      const line = lines.get(key);
      const message = line === undefined ? undefined : parseLine(line);
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the line holds the JSON value this host framed.
      return message?.kind === "result" ? (message.result as JSONValue) : null;
    },
    set: async (_runId, key, value) => {
      const bytes = encodeMessage({ id: 1, result: value });
      lines.set(key, new TextDecoder().decode(bytes).trimEnd());
    },
    getBytes: unused,
    setBytes: unused,
    delete: unused,
  };
}

/** A book, whose instances JSON would write as plain objects, losing their class. */
class Book {
  readonly title = "Dune";
}

/** `[1, <hole>, 3]`. */
function holed(): unknown[] {
  const items: unknown[] = [1];
  items[2] = 3;
  return items;
}

describe("nonJSON", () => {
  test(
    "finds nothing in any JSON value, and leaves it unchanged, `__proto__` keys included",
    { tags: ["story:11-68"] },
    () => {
      fc.assert(
        fc.property(jsonValue, (value) => {
          const before = JSON.stringify(value);

          expect(nonJSON(value)).toBeUndefined();
          expect(JSON.stringify(value)).toBe(before);
        }),
      );
    },
  );

  test(
    "names the first value that is not JSON by its JSON Pointer and what it is",
    { tags: ["story:11-68"] },
    () => {
      fc.assert(
        fc.property(
          jsonValue,
          notJSON,
          fc.array(fc.nat(), { maxLength: 4 }),
          (root, [leaf, found], steps) => {
            const [value, path] = planted(root, leaf, steps);

            expect(nonJSON(value)).toEqual({ path, found });
          },
        ),
      );
    },
  );

  test.for<[string, unknown, { path: string; found: string }]>([
    ["NaN in an object", { price: Number.NaN }, { path: "/price", found: "NaN" }],
    ["a Map in a list", { tags: [1, new Map()] }, { path: "/tags/1", found: "a Map" }],
    ["a bigint at the root", 3n, { path: "", found: "a bigint" }],
    ["an array's hole", holed(), { path: "/1", found: "undefined" }],
    ["undefined in a list", [1, undefined], { path: "/1", found: "undefined" }],
    ["a class instance", { book: new Book() }, { path: "/book", found: "an instance of Book" }],
    ["a symbol key", { [Symbol("s")]: 1 }, { path: "", found: "a symbol key" }],
  ])("refuses %s", { tags: ["story:11-68"] }, ([, value, refusal]) => {
    expect(nonJSON(value)).toEqual(refusal);
  });

  test("refuses a cycle where it closes", { tags: ["story:11-68"] }, () => {
    const inner: Record<string, unknown> = {};
    const loop = { a: inner };
    inner["back"] = loop;

    expect(nonJSON(loop)).toEqual({ path: "/a/back", found: "a cycle" });
  });

  test(
    "takes an object property holding undefined as absent, as JSON writes it",
    { tags: ["story:11-68"] },
    () => {
      expect(nonJSON({ title: "Dune", subtitle: undefined })).toBeUndefined();
    },
  );

  test(
    "takes an object without a prototype, as `JSON.parse` of `__proto__` keeps it",
    { tags: ["story:11-68"] },
    () => {
      const bare = { __proto__: null, a: 1 };

      expect(nonJSON(bare)).toBeUndefined();
      expect(nonJSON(JSON.parse('{"__proto__": {"polluted": true}}'))).toBeUndefined();
    },
  );
});

describe("a value through the Key-value store", () => {
  test(
    "comes back unchanged through `set` and `get` on the wire, `__proto__` keys included",
    { tags: ["story:11-68"] },
    async () => {
      await fc.assert(
        fc.asyncProperty(
          jsonValue.filter((value): value is StoredValue => value !== null),
          async (value) => {
            const store = new KeyValueStore(framingHost(), "run", () => {});

            await store.set("value", value);
            const back = await store.get("value");

            expect(JSON.stringify(back)).toBe(JSON.stringify(value));
            expect(back).toEqual(value);
          },
        ),
      );
    },
  );
});
