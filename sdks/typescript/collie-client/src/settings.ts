// Settings in code (ADR 0083): each scope's `settings`, typed by the
// generated `Settings`, `RunSettings`, `SessionSettings` and `PageSettings`
// and written in camelCase (ADR 0099), sent nested by the wire's
// `snake_case` segments.
import { warnDeprecatedSetting } from "./markers.ts";
import { toWire, type WirePath } from "./wire.ts";

/**
 * Each segment's wire spelling, by its camelCase one: a setting's last
 * segment, or with where its value holds fields the engine names otherwise,
 * or an area's with its own segments.
 */
export type Spelling = {
  readonly [segment: string]:
    | string
    | { readonly setting: string; readonly value: WirePath }
    | readonly [string, Spelling];
};

/**
 * Each deprecated setting's deprecation text, by its dotted wire key: what
 * the walker warns of, once per process, when a program sets one.
 */
export type DeprecatedSettings = ReadonlyMap<string, string>;

/** Settings as the wire takes them, nested by `snake_case` segment. */
export type SettingsOnWire = { [segment: string]: unknown };

/**
 * `settings` as the wire takes them: each segment `spelling` knows in its
 * wire spelling, a setting's value as written, `null` included, with the
 * fields its schema declares in their wire spelling, and a key set to
 * `undefined` left out. What `spelling` does not know is sent as
 * written, for the engine to refuse with `setting_unknown` naming the
 * closest key; the SDK checks no value the engine owns. Setting one of
 * `deprecated`'s keys, `null` included, warns once per process (ADR 0099).
 */
export function onWire(
  settings: object | undefined,
  spelling: Spelling,
  deprecated: DeprecatedSettings,
): SettingsOnWire | undefined {
  if (settings === undefined) {
    return undefined;
  }
  return spelled(settings, spelling, deprecated, "");
}

function spelled(
  settings: object,
  spelling: Spelling,
  deprecated: DeprecatedSettings,
  area: string,
): SettingsOnWire {
  const wire: SettingsOnWire = {};
  for (const [segment, value] of Object.entries(settings)) {
    if (value === undefined) {
      continue;
    }
    const known = Object.hasOwn(spelling, segment) ? spelling[segment] : undefined;
    if (known === undefined) {
      put(wire, segment, value);
    } else if (typeof known === "string") {
      warnIfDeprecated(deprecated, keyOf(area, known));
      put(wire, known, value);
    } else if ("setting" in known) {
      warnIfDeprecated(deprecated, keyOf(area, known.setting));
      put(wire, known.setting, toWire(value, known.value, {}));
    } else {
      const [name, inner] = known;
      put(
        wire,
        name,
        isTable(value) ? spelled(value, inner, deprecated, keyOf(area, name)) : value,
      );
    }
  }
  return wire;
}

/** The dotted wire key of `segment` in the area keyed `area`, `""` at the root. */
function keyOf(area: string, segment: string): string {
  return area === "" ? segment : `${area}.${segment}`;
}

function warnIfDeprecated(deprecated: DeprecatedSettings, key: string): void {
  const text = deprecated.get(key);
  if (text !== undefined) {
    warnDeprecatedSetting(key, text);
  }
}

/** Sets `wire`'s own `segment`, which an assignment to `__proto__` would not. */
function put(wire: SettingsOnWire, segment: string, value: unknown): void {
  Object.defineProperty(wire, segment, {
    value,
    enumerable: true,
    writable: true,
    configurable: true,
  });
}

/** Whether `value` is a table of segments, not a setting's value an area cannot take. */
function isTable(value: unknown): value is object {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
