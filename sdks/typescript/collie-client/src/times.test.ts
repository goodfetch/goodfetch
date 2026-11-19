// Points in time in results (ADRs 0092 and 0099; spec 606, Casing and
// times): RFC 3339 text with any offset becomes a `Date` in milliseconds, a
// finer part dropped, at the places the generated tables name, and nothing
// else changes.
import * as fc from "fast-check";
import { describe, expect, test } from "vitest";
import { pointInTime, withTimes, type TimePath } from "./times.ts";

/** A table of two types: a report with a point in time and a nested usage that holds another. */
const types: Readonly<Record<string, TimePath>> = {
  Report: {
    fields: { started_at: "time", ended_at: "time", usage: { type: "Usage" } },
  },
  Usage: { fields: { resets_at: "time" } },
};

/** `value` in two digits. */
function pad(value: number): string {
  return String(value).padStart(2, "0");
}

/** `minutes` east of UTC written as an RFC 3339 offset, `Z` for none. */
function offsetText(minutes: number, zulu: boolean): string {
  if (minutes === 0 && zulu) {
    return "Z";
  }
  const sign = minutes < 0 ? "-" : "+";
  const whole = Math.abs(minutes);
  return `${sign}${pad(Math.floor(whole / 60))}:${pad(whole % 60)}`;
}

/** The RFC 3339 text of `milliseconds` seen `offset` minutes east of UTC, with `extra` digits past the milliseconds. */
function rfc3339(milliseconds: number, offset: number, zulu: boolean, extra: string): string {
  const local = new Date(milliseconds + offset * 60_000).toISOString().slice(0, 23);
  return `${local}${extra}${offsetText(offset, zulu)}`;
}

describe("pointInTime", () => {
  test(
    "reads RFC 3339 text with any offset as the same instant in UTC, milliseconds kept and a finer part dropped",
    { tags: ["story:11-91"] },
    () => {
      fc.assert(
        fc.property(
          // From the year 1 to the 30th of December 9999, so that any offset keeps a four-digit year.
          fc.integer({
            min: Date.parse("0001-01-01T00:00:00Z"),
            max: Date.parse("9999-12-30T23:59:59.999Z"),
          }),
          fc.integer({ min: -(23 * 60 + 59), max: 23 * 60 + 59 }),
          fc.boolean(),
          fc.stringMatching(/^\d{0,6}$/),
          (milliseconds, offset, zulu, extra) => {
            expect(pointInTime(rfc3339(milliseconds, offset, zulu, extra))?.getTime()).toBe(
              milliseconds,
            );
          },
        ),
      );
    },
  );

  test.each([
    ["an offset east of UTC", "2026-11-19T14:30:00+02:00", "2026-11-19T12:30:00.000Z"],
    ["an offset west of UTC", "2026-11-19T07:30:00-05:00", "2026-11-19T12:30:00.000Z"],
    ["nanoseconds, truncated", "2026-11-19T12:30:00.123987654Z", "2026-11-19T12:30:00.123Z"],
    ["one fraction digit", "2026-11-19T12:30:00.5Z", "2026-11-19T12:30:00.500Z"],
    ["a lowercase t and z", "2026-11-19t12:30:00z", "2026-11-19T12:30:00.000Z"],
    ["a space for the T", "2026-11-19 12:30:00Z", "2026-11-19T12:30:00.000Z"],
    ["an offset crossing the day", "2026-01-01T00:30:00+01:00", "2025-12-31T23:30:00.000Z"],
    ["a year before 100", "0042-02-03T04:05:06Z", "0042-02-03T04:05:06.000Z"],
    ["the 29th of February of a leap year", "2024-02-29T00:00:00Z", "2024-02-29T00:00:00.000Z"],
  ])("reads %s", { tags: ["story:11-91"] }, (_, text, utc) => {
    expect(pointInTime(text)?.toISOString()).toBe(utc);
  });

  test.each([
    ["no offset", "2026-11-19T12:30:00"],
    ["a date alone", "2026-11-19"],
    ["a 13th month", "2026-13-01T00:00:00Z"],
    ["the 29th of February of a common year", "2026-02-29T00:00:00Z"],
    ["the 31st of April", "2026-04-31T00:00:00Z"],
    ["hour 24", "2026-11-19T24:00:00Z"],
    ["a leap second, which a Date cannot hold", "2016-12-31T23:59:60Z"],
    ["an offset of 24 hours", "2026-11-19T12:30:00+24:00"],
    ["an empty fraction", "2026-11-19T12:30:00.Z"],
    ["text around it", " 2026-11-19T12:30:00Z"],
  ])("reads no point in time from %s", { tags: ["story:11-91"] }, (_, text) => {
    expect(pointInTime(text)).toBeUndefined();
  });
});

describe("withTimes", () => {
  test(
    "turns each point in time the table names into a Date, nested ones too, and leaves every other value as sent",
    { tags: ["story:11-91"] },
    () => {
      const sent = {
        run_id: "2026-11-19T12:30:00Z",
        started_at: "2026-11-19T14:30:00.123456+02:00",
        ended_at: "2026-11-19T12:31:00Z",
        usage: { resets_at: null, requests: 3 },
        carbon: "2026-11-19T12:30:00Z",
      };

      const read = withTimes(sent, { type: "Report" }, types);

      expect(read).toEqual({
        run_id: "2026-11-19T12:30:00Z",
        started_at: new Date("2026-11-19T12:30:00.123Z"),
        ended_at: new Date("2026-11-19T12:31:00.000Z"),
        usage: { resets_at: null, requests: 3 },
        carbon: "2026-11-19T12:30:00Z",
      });
    },
  );

  test(
    "turns the points in time of a list's items and a map's values",
    { tags: ["story:11-91"] },
    () => {
      const read = withTimes(
        JSON.parse(
          `{"rows": [{"resets_at": "2026-11-19T12:30:00Z"}, {"resets_at": null}],
            "by_kind": {"__proto__": {"resets_at": "2026-11-19T13:30:00+01:00"}}}`,
        ),
        {
          fields: {
            rows: { items: { type: "Usage" } },
            by_kind: { values: { type: "Usage" } },
          },
        },
        types,
      );

      expect(read).toMatchObject({
        rows: [{ resets_at: new Date("2026-11-19T12:30:00Z") }, { resets_at: null }],
      });
      const byKind: unknown = Object.getOwnPropertyDescriptor(read, "by_kind")?.value;
      expect(Object.getOwnPropertyDescriptor(byKind, "__proto__")?.value).toEqual({
        resets_at: new Date("2026-11-19T12:30:00Z"),
      });
    },
  );

  test(
    "keys a map's values by the keys sent, `__proto__` included, as own entries",
    { tags: ["story:11-91"] },
    () => {
      const sent: unknown = JSON.parse('{"__proto__": "2026-11-19T12:30:00Z"}');

      const read = withTimes(sent, { values: "time" }, types);

      expect(Object.getPrototypeOf(read)).toBe(Object.prototype);
      expect(Object.getOwnPropertyDescriptor(read, "__proto__")?.value).toEqual(
        new Date("2026-11-19T12:30:00Z"),
      );
    },
  );

  test(
    "leaves a value the engine sent in another shape as sent: a number, an unreadable text, a type the table lacks",
    { tags: ["story:11-91"] },
    () => {
      const sent = { started_at: 1_763_555_400, ended_at: "soon", usage: { resets_at: "x" } };

      const read = withTimes(
        sent,
        { fields: { started_at: "time", ended_at: "time", usage: { type: "Missing" } } },
        types,
      );

      expect(read).toEqual({
        started_at: 1_763_555_400,
        ended_at: "soon",
        usage: { resets_at: "x" },
      });
    },
  );
});
