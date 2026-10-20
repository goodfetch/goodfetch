// The host's logger (ADR 0078): `console`, pino and winston fit it. With
// none, the SDK is silent; it never writes to stdout or stderr itself.

/** The host program's logger, which the SDK writes through and nowhere else. */
export interface Logger {
  debug(message: string, fields?: Readonly<Record<string, unknown>>): void;
  info(message: string, fields?: Readonly<Record<string, unknown>>): void;
  warn(message: string, fields?: Readonly<Record<string, unknown>>): void;
  error(message: string, fields?: Readonly<Record<string, unknown>>): void;
}
