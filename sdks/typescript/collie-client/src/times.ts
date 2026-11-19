// Points in time in results (ADRs 0092 and 0099): the engine sends RFC 3339
// text, and a program reads a `Date` in UTC, to the millisecond. The
// generated tables (`generated/times.ts`) say where each result holds one;
// sending needs nothing, since `Date.toJSON` writes RFC 3339 in UTC.

/**
 * Where a value holds points in time: `"time"` for one, a named type of the
 * table, an object's fields, a list's items or a map's values.
 */
export type TimePath =
  | "time"
  | { readonly type: string }
  | { readonly fields: Readonly<Record<string, TimePath>> }
  | { readonly items: TimePath }
  | { readonly values: TimePath };

/** A Product's table: the path of each type that holds a point in time, and of each operation's result that does. */
export interface Times<Operation extends string> {
  readonly types: Readonly<Record<string, TimePath>>;
  readonly results: Readonly<Partial<Record<Operation, TimePath>>>;
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
  const y = Number(part(1));
  const mo = Number(part(2));
  const d = Number(part(3));
  const h = Number(part(4));
  const mi = Number(part(5));
  const s = Number(part(6));
  const offsetHours = Number(part(9));
  const offsetMinutes = Number(part(10));
  const offset = (offsetHours * 60 + offsetMinutes) * (part(8) === "-" ? -1 : 1);
  if (
    mo < 1 ||
    mo > 12 ||
    d < 1 ||
    d > daysIn(y, mo) ||
    h > 23 ||
    mi > 59 ||
    s > 59 ||
    offsetHours > 23 ||
    offsetMinutes > 59
  ) {
    return undefined;
  }
  const milliseconds = Number(part(7).slice(0, 3).padEnd(3, "0"));
  const date = new Date(0);
  // `Date.UTC` reads a year below 100 as 19xx; these setters do not.
  date.setUTCFullYear(y, mo - 1, d);
  date.setUTCHours(h, mi - offset, s, milliseconds);
  return Number.isNaN(date.getTime()) ? undefined : date;
}

/** Whether `value` is a plain object of a JSON result: not null, not a list. */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
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

/** `value` with `path`'s points in time read, lists and objects changed in place. */
function read(value: unknown, path: TimePath, types: Readonly<Record<string, TimePath>>): unknown {
  if (path === "time") {
    return typeof value === "string" ? (pointInTime(value) ?? value) : value;
  }
  if ("type" in path) {
    const named = Object.hasOwn(types, path.type) ? types[path.type] : undefined;
    return named === undefined ? value : read(value, named, types);
  }
  if ("items" in path) {
    if (Array.isArray(value)) {
      value.forEach((item: unknown, index) => {
        value[index] = read(item, path.items, types);
      });
    }
    return value;
  }
  if (!isRecord(value)) {
    return value;
  }
  const entries: Iterable<[string, TimePath]> =
    "values" in path
      ? Object.keys(value).map((key) => [key, path.values])
      : Object.entries(path.fields).filter(([key]) => Object.hasOwn(value, key));
  for (const [key, inner] of entries) {
    const before = value[key];
    const after = read(before, inner, types);
    if (after !== before) {
      setOwn(value, key, after);
    }
  }
  return value;
}

/**
 * `value`, a result as the engine sent it, with each point in time `path`
 * names read as a `Date`. A value in another shape stays as sent: results are
 * not validated beyond the protocol version check.
 */
export function withTimes(
  value: unknown,
  path: TimePath,
  types: Readonly<Record<string, TimePath>>,
): unknown {
  return read(value, path, types);
}
