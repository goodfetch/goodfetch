// The engine's names and points in time (ADRs 0092 and 0099): a program
// reads and writes each field the schemas declare in camelCase, and each
// point in time as a `Date` in UTC, to the millisecond, while the engine
// speaks `snake_case` and RFC 3339 text. The generated tables
// (`generated/wire.ts`) say, for each type, each operation's params and
// result and each event's payload, which fields the engine names otherwise
// and where a point in time sits. Only those fields are renamed: the keys of
// a map or of a free value are data and keep their spelling, `__proto__`
// included, and a field no table knows passes as it is, since results are
// neither validated nor stripped (ADR 0082). Sending a point in time needs
// nothing, since `Date.toJSON` writes RFC 3339 in UTC.
import { isRecord } from "./framing.ts";

/**
 * Where a value holds fields to rename or points in time: `"time"` for one, a
 * named type of the table, an object's fields, a list's items or a map's
 * values.
 */
export type WirePath =
  | "time"
  | { readonly type: string }
  | { readonly fields: Readonly<Record<string, WireField>> }
  | { readonly items: WirePath }
  | { readonly values: WirePath };

/**
 * A field of a `WirePath`, keyed by the engine's name: its camelCase name, or
 * that name and where its value holds something.
 */
export type WireField = string | readonly [string, WirePath];

/** The named types of a table, which a `WirePath`'s `type` names. */
export type WireTypes = Readonly<Record<string, WirePath>>;

/**
 * A Product's table: the path of each type that holds something, and of each
 * operation's params and result, and each event's payload, that does; `Type`
 * names the table's types.
 */
export interface Wire<
  Operation extends string,
  Event extends string = string,
  Type extends string = string,
> {
  readonly types: Readonly<Record<Type, WirePath>>;
  readonly params: Readonly<Partial<Record<Operation, WirePath>>>;
  readonly results: Readonly<Partial<Record<Operation, WirePath>>>;
  readonly events: Readonly<Partial<Record<Event, WirePath>>>;
}

const RFC_3339 =
  /^(\d{4})-(\d{2})-(\d{2})[Tt ](\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?(?:[Zz]|([+-])(\d{2}):(\d{2}))$/;

/** Whether `year` is a leap year of the proleptic Gregorian calendar. */
function isLeapYear(year: number): boolean {
  return (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
}

/** The number of days in `month` (1 to 12) of `year`. */
function daysIn(year: number, month: number): number {
  if (month === 2) {
    return isLeapYear(year) ? 29 : 28;
  }
  return month === 4 || month === 6 || month === 9 || month === 11 ? 30 : 31;
}

/**
 * The instant RFC 3339 `text` names, in UTC, its fraction cut to
 * milliseconds; `undefined` when `text` is no RFC 3339 date-time or names a
 * leap second, which a `Date` cannot hold.
 */
export function pointInTime(text: string): Date | undefined {
  const parts = RFC_3339.exec(text);
  if (parts === null) {
    return undefined;
  }
  const part = (index: number): string => parts[index] ?? "";
  const year = Number(part(1));
  const month = Number(part(2));
  const day = Number(part(3));
  const hour = Number(part(4));
  const minute = Number(part(5));
  const second = Number(part(6));
  const offsetHours = Number(part(9));
  const offsetMinutes = Number(part(10));
  const offset = (offsetHours * 60 + offsetMinutes) * (part(8) === "-" ? -1 : 1);
  if (
    month < 1 ||
    month > 12 ||
    day < 1 ||
    day > daysIn(year, month) ||
    hour > 23 ||
    minute > 59 ||
    second > 59 ||
    offsetHours > 23 ||
    offsetMinutes > 59
  ) {
    return undefined;
  }
  const milliseconds = Number(part(7).slice(0, 3).padEnd(3, "0"));
  const date = new Date(0);
  // `Date.UTC` reads a year below 100 as 19xx; these setters do not.
  date.setUTCFullYear(year, month - 1, day);
  date.setUTCHours(hour, minute - offset, second, milliseconds);
  return Number.isNaN(date.getTime()) ? undefined : date;
}

/** Sets `key` of `target` to `value` as an own entry, `__proto__` included. */
function setOwn(target: object, key: string, value: unknown): void {
  Object.defineProperty(target, key, {
    value,
    enumerable: true,
    writable: true,
    configurable: true,
  });
}

/** A field's name and the path of its value. */
function partsOf(field: WireField): readonly [string, WirePath | undefined] {
  return typeof field === "string" ? [field, undefined] : field;
}

/** Each field of a path by its camelCase name: its engine name and value's path. */
const byCamel = new WeakMap<
  Readonly<Record<string, WireField>>,
  ReadonlyMap<string, readonly [string, WirePath | undefined]>
>();

function camelFields(
  fields: Readonly<Record<string, WireField>>,
): ReadonlyMap<string, readonly [string, WirePath | undefined]> {
  let known = byCamel.get(fields);
  if (known === undefined) {
    known = new Map(
      Object.entries(fields).map(([wire, field]) => {
        const [name, inner] = partsOf(field);
        return [name, [wire, inner]] as const;
      }),
    );
    byCamel.set(fields, known);
  }
  return known;
}

/** The path a `type` path names, `undefined` when the table holds nothing for it. */
function named(path: { readonly type: string }, types: WireTypes): WirePath | undefined {
  return Object.hasOwn(types, path.type) ? types[path.type] : undefined;
}

/**
 * `value`, as the engine sent it, with each field `path` names in camelCase
 * and each point in time read as a `Date`, in new lists and objects. A value
 * in another shape stays as sent: what the engine sends is not validated
 * beyond the protocol version check.
 */
export function fromWire(value: unknown, path: WirePath | undefined, types: WireTypes): unknown {
  if (path === undefined) {
    return value;
  }
  if (path === "time") {
    return typeof value === "string" ? (pointInTime(value) ?? value) : value;
  }
  if ("type" in path) {
    return fromWire(value, named(path, types), types);
  }
  if ("items" in path) {
    return Array.isArray(value)
      ? value.map((item: unknown) => fromWire(item, path.items, types))
      : value;
  }
  if (!isRecord(value)) {
    return value;
  }
  const read: Record<string, unknown> = {};
  for (const [key, item] of Object.entries(value)) {
    if ("values" in path) {
      setOwn(read, key, fromWire(item, path.values, types));
      continue;
    }
    const field = Object.hasOwn(path.fields, key) ? path.fields[key] : undefined;
    if (field === undefined) {
      setOwn(read, key, item);
    } else {
      const [name, inner] = partsOf(field);
      setOwn(read, name, fromWire(item, inner, types));
    }
  }
  return read;
}

/** Whether `value` is an object a program wrote as a literal, not a class's instance. */
function isPlain(value: unknown): value is Readonly<Record<string, unknown>> {
  if (!isRecord(value)) {
    return false;
  }
  const prototype: unknown = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

/**
 * `value`, as a program wrote it, with each field `path` names in the
 * engine's spelling, in new lists and objects; the program's own are left as
 * they are. A field no table knows is sent as written, for the engine to
 * refuse.
 */
export function toWire(value: unknown, path: WirePath | undefined, types: WireTypes): unknown {
  if (path === undefined || path === "time") {
    return value;
  }
  if ("type" in path) {
    return toWire(value, named(path, types), types);
  }
  if ("items" in path) {
    return Array.isArray(value)
      ? value.map((item: unknown) => toWire(item, path.items, types))
      : value;
  }
  if (!isPlain(value)) {
    return value;
  }
  const sent: Record<string, unknown> = {};
  const known = "fields" in path ? camelFields(path.fields) : undefined;
  for (const [key, item] of Object.entries(value)) {
    if (known === undefined) {
      setOwn(sent, key, toWire(item, "values" in path ? path.values : undefined, types));
      continue;
    }
    const field = known.get(key);
    if (field === undefined) {
      setOwn(sent, key, item);
    } else {
      const [wire, inner] = field;
      setOwn(sent, wire, toWire(item, inner, types));
    }
  }
  return sent;
}
