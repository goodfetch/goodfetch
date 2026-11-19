// The level of the engine's logs a Connection asks for, read from the host's
// logger (ADR 0078): its `level` when it is one of the engine's level names,
// `info` for a logger that names none or another, and `off` with no logger;
// and what an engine's log event the logger cannot take costs: that event.
import { describe, expect, test } from "vitest";
import { logEngineEvent, logLevelOf, type Logger } from "./logger.ts";

/** Records nothing. */
function ignore(): void {}

/** Fails as a host's logger may. */
function fail(): never {
  throw new Error("the host's logger failed");
}

/** A logger that records nothing, with `level` as given. */
function loggerAt(level: unknown): Logger {
  return { debug: ignore, info: ignore, warn: ignore, error: ignore, level };
}

describe("the log level a logger records", () => {
  test.each(["off", "error", "warn", "info", "debug", "trace"] as const)(
    "is the logger's own level %s",
    { tags: ["story:11-26"] },
    (level) => {
      expect(logLevelOf(loggerAt(level))).toBe(level);
    },
  );

  test.each([
    ["winston's verbose", "verbose"],
    ["pino's silent", "silent"],
    ["a number", 30],
    ["a method", () => "debug"],
  ])(
    "is info for a level that is no engine level name: %s",
    { tags: ["story:11-26"] },
    (_, level) => {
      expect(logLevelOf(loggerAt(level))).toBe("info");
    },
  );

  test("is info for a logger with no level, such as console", { tags: ["story:11-26"] }, () => {
    expect(logLevelOf({ debug: ignore, info: ignore, warn: ignore, error: ignore })).toBe("info");
  });

  test("is off with no logger", { tags: ["story:11-26"] }, () => {
    expect(logLevelOf(undefined)).toBe("off");
  });
});

describe("an engine's log event handed to the logger", () => {
  test("is lost alone when the logger throws", { tags: ["story:11-25"] }, () => {
    const throwing: Logger = { debug: fail, info: fail, warn: fail, error: fail };

    expect(() =>
      logEngineEvent(throwing, { level: "error", target: "goodfetch::run", message: "lost" }),
    ).not.toThrow();
  });

  test.each([
    ["an event at off", { level: "off", target: "goodfetch::run", message: "unread" }],
    ["no params", undefined],
    ["null fields", { level: "info", target: "goodfetch::run", message: "m", fields: null }],
  ])("reaches no method of the logger for %s", { tags: ["story:11-25"] }, (_, params) => {
    const called: string[] = [];
    const record = (method: string) => (): void => {
      called.push(method);
    };
    const logger: Logger = {
      debug: record("debug"),
      info: record("info"),
      warn: record("warn"),
      error: record("error"),
    };

    logEngineEvent(logger, params);

    expect(called).toEqual([]);
  });
});
