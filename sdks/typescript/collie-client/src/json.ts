// JSON values, and the check that a value a program sends is one (ADR 0082,
// G3): only JSON crosses, and a value that is not raises from the call that
// sent it, naming its field path, rather than being turned into a string or
// dropped, which would read back as a different value. User data keeps its
// keys as given, `__proto__` included.

/** A JSON value. */
export type JSONValue =
  | string
  | number
  | boolean
  | null
  | readonly JSONValue[]
  | { readonly [key: string]: JSONValue };

/** A JSON object. */
export type JSONObject = { readonly [key: string]: JSONValue };

/** Where a value that is not JSON sits, and what it is. */
export interface NonJSON {
  /** Its JSON Pointer (RFC 6901), empty at the root. */
  readonly path: string;
  /** What it is: `NaN`, `a bigint`, `a Map`, `an instance of Book`, `a cycle`. */
  readonly found: string;
}

/**
 * The first value in `value`, depth first, that is not JSON, or `undefined`
 * when it is all JSON. A plain object's property holding `undefined` is
 * absent, as JSON writes it; an object without a prototype is plain, as
 * `JSON.parse` gives for a `__proto__` key.
 */
export function nonJSON(value: unknown): NonJSON | undefined {
  return firstNonJSON(value, "", new Set());
}

function firstNonJSON(value: unknown, path: string, open: Set<object>): NonJSON | undefined {
  switch (typeof value) {
    case "string":
    case "boolean":
      return undefined;
    case "number":
      return Number.isFinite(value) ? undefined : { path, found: String(value) };
    case "bigint":
      return { path, found: "a bigint" };
    case "symbol":
      return { path, found: "a symbol" };
    case "function":
      return { path, found: "a function" };
    case "undefined":
      return { path, found: "undefined" };
    case "object":
      break;
  }
  return value === null ? undefined : objectNonJSON(value, path, open);
}

function objectNonJSON(value: object, path: string, open: Set<object>): NonJSON | undefined {
  if (open.has(value)) {
    return { path, found: "a cycle" };
  }
  if (!isPlain(value)) {
    return { path, found: describe(value) };
  }
  if (Object.getOwnPropertySymbols(value).length > 0) {
    return { path, found: "a symbol key" };
  }
  // A list's hole reads as `undefined`, which it refuses; an object's
  // property holding `undefined` is absent.
  const members: readonly (readonly [string, unknown])[] = Array.isArray(value)
    ? Array.from(value, (item: unknown, index): readonly [string, unknown] => [String(index), item])
    : Object.entries(value).filter(([, field]: [string, unknown]) => field !== undefined);
  open.add(value);
  try {
    for (const [key, member] of members) {
      const found = firstNonJSON(member, `${path}/${escaped(key)}`, open);
      if (found) {
        return found;
      }
    }
    return undefined;
  } finally {
    open.delete(value);
  }
}

/** Whether `value` is a plain array or a plain object, which JSON holds as given. */
function isPlain(value: object): boolean {
  const prototype: unknown = Object.getPrototypeOf(value);
  if (Array.isArray(value)) {
    return prototype === Array.prototype;
  }
  return prototype === Object.prototype || prototype === null;
}

/** How a refusal names an object that is not JSON: `a Map`, `an instance of Book`. */
function describe(value: object): string {
  const name = Object.prototype.toString.call(value).slice("[object ".length, -1);
  if (name !== "Object") {
    return `a ${name}`;
  }
  const constructor: unknown = Object.getPrototypeOf(value)?.constructor;
  return typeof constructor === "function" && constructor.name !== ""
    ? `an instance of ${constructor.name}`
    : "an object with a prototype";
}

/** `key` as a JSON Pointer's reference token (RFC 6901). */
function escaped(key: string): string {
  return key.replaceAll("~", "~0").replaceAll("/", "~1");
}
