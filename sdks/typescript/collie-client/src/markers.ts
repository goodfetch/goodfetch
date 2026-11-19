// ADR 0030's run-time warning for a deprecated or Experimental catalog entry
// (ADR 0099). Each generated member of one calls it, and each hand-written
// member and listener reads its entry's markers from the generated
// registries through `warnEntry`; it warns once per
// process per name and marker, through Node's own warning channel,
// `process.emitWarning`, the host's (ADR 0078): a host listens with
// `process.on("warning")` and silences them with `--no-deprecation` or
// `--no-warnings`, and the SDK never prints them itself. The type-level
// markers, TSDoc's `@deprecated` and `@experimental`, are the member's own.

const warned = new Set<string>();

/** An entry's markers, as the catalog declares them: the generated registries of operations and events hold one for each. */
export interface EntryMarkers {
  readonly experimental: boolean;
  /** Its deprecation's one text, or `undefined` when it is not deprecated. */
  readonly deprecated: string | undefined;
}

/**
 * Warns, the first time in the process, of each marker `entries` gives the
 * entry `name`: what a hand-written member serving it, or a listener of an
 * event, calls first, as a generated member calls `warnExperimental` and
 * `warnDeprecated`.
 */
export function warnEntry<Name extends string>(
  entries: Readonly<Record<Name, EntryMarkers>>,
  name: Name,
): void {
  const markers = entries[name];
  if (markers.experimental) {
    warnExperimental(name);
  }
  if (markers.deprecated !== undefined) {
    warnDeprecated(name, markers.deprecated);
  }
}

/**
 * Warns, the first time in the process, that the entry `name` is deprecated.
 * `text` is its deprecation's one text, as its `@deprecated` renders it.
 */
export function warnDeprecated(name: string, text: string): void {
  warnOnce(`deprecated ${name}`, `\`${name}\` is deprecated. ${text}`, {
    type: "DeprecationWarning",
    code: "GOODFETCH_DEPRECATED",
  });
}

/** Warns, the first time in the process, that the entry `name` is Experimental. */
export function warnExperimental(name: string): void {
  warnOnce(
    `experimental ${name}`,
    `\`${name}\` is Experimental: outside semver until it is promoted (ADR 0030).`,
    { type: "ExperimentalWarning", code: "GOODFETCH_EXPERIMENTAL" },
  );
}

function warnOnce(key: string, message: string, options: { type: string; code: string }): void {
  if (warned.has(key)) {
    return;
  }
  warned.add(key);
  process.emitWarning(message, options);
}
