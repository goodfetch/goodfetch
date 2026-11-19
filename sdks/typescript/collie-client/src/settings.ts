// Settings in code (ADR 0083): each scope's `settings`, typed by the
// generated `Settings`, `RunSettings`, `SessionSettings` and `PageSettings`
// and written in camelCase (ADR 0099), sent nested by the wire's
// `snake_case` segments.

/** Each segment's wire spelling, by its camelCase one: a setting's last segment, or an area's with its own segments. */
export type Spelling = { readonly [segment: string]: string | readonly [string, Spelling] };

/** Settings as the wire takes them, nested by `snake_case` segment. */
export type SettingsOnWire = { [segment: string]: unknown };

/**
 * `settings` as the wire takes them: each segment `spelling` knows in its
 * wire spelling, a setting's value as written, `null` included, and a key
 * set to `undefined` left out. What `spelling` does not know is sent as
 * written, for the engine to refuse with `setting_unknown` naming the
 * closest key; the SDK checks no value the engine owns.
 */
export function onWire(
  settings: object | undefined,
  spelling: Spelling,
): SettingsOnWire | undefined {
  if (settings === undefined) {
    return undefined;
  }
  return spelled(settings, spelling);
}

function spelled(settings: object, spelling: Spelling): SettingsOnWire {
  const wire: SettingsOnWire = {};
  for (const [segment, value] of Object.entries(settings)) {
    if (value === undefined) {
      continue;
    }
    const known = Object.hasOwn(spelling, segment) ? spelling[segment] : undefined;
    if (known === undefined) {
      put(wire, segment, value);
    } else if (typeof known === "string") {
      put(wire, known, value);
    } else {
      const [name, inner] = known;
      put(wire, name, isTable(value) ? spelled(value, inner) : value);
    }
  }
  return wire;
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
