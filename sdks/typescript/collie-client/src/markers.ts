// ADR 0030's run-time warning for a deprecated or Experimental catalog entry
// (ADR 0099). Each generated member of one calls it; it warns once per
// process per name and marker, through Node's own warning channel,
// `process.emitWarning`, the host's (ADR 0078): a host listens with
// `process.on("warning")` and silences them with `--no-deprecation` or
// `--no-warnings`, and the SDK never prints them itself. The type-level
// markers, TSDoc's `@deprecated` and `@experimental`, are the member's own.

const warned = new Set<string>();

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
