// The host's logger (ADR 0078): `console`, pino and winston fit it. With
// none, the SDK is silent; it never writes to stdout or stderr itself. The
// level of the engine's logs a Connection receives follows what the logger
// records: its `level` when that is one of the engine's level names, `info`
// otherwise, and `off` with no logger.
import type { LogEvent, LogLevel } from "./generated/index.ts";

/** The host program's logger, which the SDK writes through and nowhere else. */
export interface Logger {
  debug(message: string, fields?: Readonly<Record<string, unknown>>): void;
  info(message: string, fields?: Readonly<Record<string, unknown>>): void;
  warn(message: string, fields?: Readonly<Record<string, unknown>>): void;
  error(message: string, fields?: Readonly<Record<string, unknown>>): void;
  /**
   * The lowest level it records, as pino's and winston's `level` names it.
   * When it is one of the engine's level names (`off`, `error`, `warn`,
   * `info`, `debug`, `trace`), the engine sends the Connection its logs at
   * that level and above; any other value, or none, as `console` has,
   * receives them from `info`. Read at `connect()` and again when a Run
   * starts, never polled.
   */
  readonly level?: unknown;
}

/** The engine's level names, which a logger's `level` may be, each once. */
const LOG_LEVELS = {
  off: true,
  error: true,
  warn: true,
  info: true,
  debug: true,
  trace: true,
} as const satisfies Record<LogLevel, true>;

function isLogLevel(value: unknown): value is LogLevel {
  return typeof value === "string" && Object.hasOwn(LOG_LEVELS, value);
}

/**
 * The level of the engine's logs a Connection whose host logs through
 * `logger` asks for: the logger's `level` when it is one of the engine's
 * level names, `info` when it is another or none, and `off` with no logger.
 */
export function logLevelOf(logger: Logger | undefined): LogLevel {
  if (logger === undefined) {
    return "off";
  }
  const { level } = logger;
  return isLogLevel(level) ? level : "info";
}

/**
 * Hands one of the engine's log events, a `$/log` notification's params, to
 * `logger` at its level, `trace` at `debug`, with its target, its fields and
 * how many events before it were dropped. A logger that throws, or params
 * that are no log event, lose the event alone: the Connection goes on, and
 * the SDK has nowhere else to say so.
 */
export function logEngineEvent(logger: Logger | undefined, params: unknown): void {
  if (logger === undefined) {
    return;
  }
  try {
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine messages are not validated beyond `hello`'s protocol version (STYLE.md): a `$/log` notification carries a log event.
    const { level, target, message, fields, dropped } = params as LogEvent;
    const context = {
      source: "engine",
      target,
      ...(fields === undefined || Object.keys(fields).length === 0 ? {} : { fields }),
      ...(dropped === undefined || dropped === 0 ? {} : { dropped }),
    };
    switch (level) {
      case "off":
        return;
      case "error":
        logger.error(message, context);
        return;
      case "warn":
        logger.warn(message, context);
        return;
      case "info":
        logger.info(message, context);
        return;
      case "debug":
      case "trace":
        logger.debug(message, context);
        return;
      default: {
        const unknown: never = level;
        return unknown;
      }
    }
  } catch {
    // The host's logger failed, or the engine sent no log event: the event
    // is lost, and the Connection goes on.
  }
}
