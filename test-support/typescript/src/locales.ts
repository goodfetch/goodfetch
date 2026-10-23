// The shared table of locales and time zones (ADR 0073), read from
// `test-support/fixtures/locales.json`, the one file Rust and Python read
// too, so Period Limits, Schedules and formatting take the same zone cases
// in every language.
import type { ValidateFunction } from "ajv/dist/2020.js";
import { ajv, committedSchema, sharedFixture } from "./schemas.ts";

/** One shared time zone, with the days its daylight saving time starts and ends on when a test needs them. */
export interface TimeZoneCase {
  /** The zone's IANA name, such as `Europe/Paris`. */
  name: string;
  /** The day in 2026, `YYYY-MM-DD`, whose local clock skips an hour (the gap). */
  dst_gap?: string | undefined;
  /** The day in 2026, `YYYY-MM-DD`, whose local clock repeats an hour (the overlap). */
  dst_overlap?: string | undefined;
}

interface LocaleTable {
  locales: string[];
  time_zones: TimeZoneCase[];
}

const isLocaleTable: ValidateFunction<LocaleTable> = ajv.compile<LocaleTable>(
  committedSchema("locales.schema.json"),
);

/** The shared locales' BCP 47 tags: `en-US`, `de-DE`, `fr-FR`, `tr-TR`, `ar-EG`, `hi-IN` and `ja-JP`. */
export function locales(): string[] {
  return sharedFixture("locales.json", isLocaleTable).locales;
}

/**
 * The shared time zones: `UTC`, `America/St_Johns`, `Asia/Kathmandu`,
 * `Pacific/Chatham`, `Australia/Lord_Howe`, and `Europe/Paris` and
 * `America/New_York` with their days of daylight saving time's gap and overlap.
 */
export function timeZones(): TimeZoneCase[] {
  return sharedFixture("locales.json", isLocaleTable).time_zones;
}
