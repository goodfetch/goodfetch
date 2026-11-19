// The engine's names and points in time (ADRs 0092 and 0099; spec 606,
// Casing and times): each declared field the generated tables name is read
// in camelCase and sent in the engine's spelling, RFC 3339 text with any
// offset becomes a `Date` in milliseconds, a finer part dropped, and data
// keys and every other value stay as they are.
import * as fc from "fast-check";
import { describe, expect, test } from "vitest";
import { fromWire, pointInTime, toWire, type WireTypes } from "./wire.ts";

/**
 * A table of three types: a report with points in time, a nested usage that
 * holds another, and a page whose metadata map's keys are data.
 */
const types: WireTypes = {
  Report: {
    fields: {
      run_id: "runId",
      started_at: ["startedAt", "time"],
      ended_at: ["endedAt", "time"],
      usage: ["usage", { type: "Usage" }],
    },
  },
  Usage: { fields: { resets_at: ["resetsAt", "time"] } },
  Page: { fields: { final_url: "finalURL", by_kind: ["byKind", { values: { type: "Usage" } }] } },
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

/** The own enumerable keys of `value`, or none when it is no object. */
function keysOf(value: unknown): readonly string[] {
  return typeof value === "object" && value !== null ? Object.keys(value) : [];
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

describe("fromWire", () => {
  test(
    "names each declared field in camelCase and turns each point in time into a Date, nested ones too, every other value as sent",
    { tags: ["story:11-91"] },
    () => {
      const sent = {
        run_id: "2026-11-19T12:30:00Z",
        started_at: "2026-11-19T14:30:00.123456+02:00",
        ended_at: "2026-11-19T12:31:00Z",
        usage: { resets_at: null, requests: 3 },
        carbon_grams: "2026-11-19T12:30:00Z",
      };

      const read = fromWire(sent, { type: "Report" }, types);

      expect(read).toStrictEqual({
        runId: "2026-11-19T12:30:00Z",
        startedAt: new Date("2026-11-19T12:30:00.123Z"),
        endedAt: new Date("2026-11-19T12:31:00.000Z"),
        usage: { resetsAt: null, requests: 3 },
        carbon_grams: "2026-11-19T12:30:00Z",
      });
    },
  );

  test(
    "follows a list's items and a map's values, and keeps a map's keys byte for byte, `__proto__` included, as own entries",
    { tags: ["story:11-91"] },
    () => {
      const read = fromWire(
        JSON.parse(
          `{"rows": [{"resets_at": "2026-11-19T12:30:00Z"}, {"resets_at": null}],
            "by_kind": {"__proto__": {"resets_at": "2026-11-19T13:30:00+01:00"}, "snake_key": {}}}`,
        ),
        {
          fields: {
            rows: ["rows", { items: { type: "Usage" } }],
            by_kind: ["byKind", { values: { type: "Usage" } }],
          },
        },
        types,
      );

      expect(read).toMatchObject({
        rows: [{ resetsAt: new Date("2026-11-19T12:30:00Z") }, { resetsAt: null }],
      });
      const byKind: unknown = Object.getOwnPropertyDescriptor(read, "byKind")?.value;
      expect(Object.getPrototypeOf(byKind)).toBe(Object.prototype);
      expect(keysOf(byKind)).toStrictEqual(["__proto__", "snake_key"]);
      expect(Object.getOwnPropertyDescriptor(byKind, "__proto__")?.value).toStrictEqual({
        resetsAt: new Date("2026-11-19T12:30:00Z"),
      });
    },
  );

  test(
    "leaves a value the engine sent in another shape as sent: a number, an unreadable text, a type the table lacks",
    { tags: ["story:11-91"] },
    () => {
      const sent = { started_at: 1_763_555_400, ended_at: "soon", usage: { resets_at: "x" } };

      const read = fromWire(
        sent,
        {
          fields: {
            started_at: ["startedAt", "time"],
            ended_at: ["endedAt", "time"],
            usage: ["usage", { type: "Missing" }],
          },
        },
        types,
      );

      expect(read).toStrictEqual({
        startedAt: 1_763_555_400,
        endedAt: "soon",
        usage: { resets_at: "x" },
      });
    },
  );
});

describe("toWire", () => {
  test("sends each declared field in the engine's spelling and the program's data keys as written", () => {
    const usage = { resetsAt: new Date("2026-11-19T12:30:00Z") };
    // `Object.fromEntries` makes `__proto__` an own key, as `JSON.parse` does.
    const byKindWritten = Object.fromEntries([
      ["__proto__", {}],
      ["camelKey", usage],
    ]);
    const written = { finalURL: "https://a.test/", byKind: byKindWritten };

    const sent = toWire(written, { type: "Page" }, types);

    expect(keysOf(sent)).toStrictEqual(["final_url", "by_kind"]);
    const byKind: unknown = Object.getOwnPropertyDescriptor(sent, "by_kind")?.value;
    expect(keysOf(byKind)).toStrictEqual(["__proto__", "camelKey"]);
    expect(Object.getOwnPropertyDescriptor(byKind, "camelKey")?.value).toStrictEqual({
      resets_at: usage.resetsAt,
    });
    expect(usage).toStrictEqual({ resetsAt: new Date("2026-11-19T12:30:00Z") });
  });

  test("reads back what it sends", () => {
    const written = {
      runId: "r1",
      startedAt: new Date("2026-11-19T12:30:00Z"),
      usage: { resetsAt: null },
    };

    const sent: unknown = JSON.parse(JSON.stringify(toWire(written, { type: "Report" }, types)));

    expect(sent).toStrictEqual({
      run_id: "r1",
      started_at: "2026-11-19T12:30:00.000Z",
      usage: { resets_at: null },
    });
    expect(fromWire(sent, { type: "Report" }, types)).toStrictEqual(written);
  });

  test("leaves a class's instance as it is", () => {
    const date = new Date("2026-11-19T12:30:00Z");

    expect(toWire(date, { type: "Usage" }, types)).toBe(date);
  });
});
