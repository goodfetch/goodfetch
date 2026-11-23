// A host logger for the SDK tests that play a script on the Scripted engine:
// the engine's stderr lines reach the host's logger as warnings, and a failed
// play writes one, so a test that saw no warning saw its script play through.
import type { Logger } from "@goodfetch/collie-client";

/** A host logger that keeps each warning, where the engine's stderr lines go, and each error. */
export function recordingLogger(): {
  readonly logger: Logger;
  readonly warned: string[];
  readonly errored: string[];
} {
  const warned: string[] = [];
  const errored: string[] = [];
  const logger: Logger = {
    // It asks for none of the engine's log events: `hello` says `off`, as the scripts expect.
    level: "off",
    debug: () => {},
    info: () => {},
    warn: (message) => {
      warned.push(message);
    },
    error: (message) => {
      errored.push(message);
    },
  };
  return { logger, warned, errored };
}

/** What a start logged: its level, message and fields. */
export type StartLogged = readonly [level: "info" | "warn", message: string, fields: unknown];

/**
 * A host logger that keeps what the SDK logs at info and warning, such as a
 * start that resumed or found its Run ended, and apart the engine's stderr
 * lines, which a failed play writes. It leaves out the engine `connect()`
 * reached, logged at info with its `EngineInfo`.
 */
export function startRecordingLogger(): {
  readonly logger: Logger;
  readonly logged: StartLogged[];
  readonly stderr: string[];
} {
  const logged: StartLogged[] = [];
  const stderr: string[] = [];
  const logger: Logger = {
    // It asks for none of the engine's log events: `hello` says `off`, as the scripts expect.
    level: "off",
    debug: () => {},
    info: (message, fields) => {
      if (fields?.["engine"] === undefined) {
        logged.push(["info", message, fields]);
      }
    },
    warn: (message, fields) => {
      if (fields?.["source"] === "engine stderr") {
        stderr.push(message);
      } else {
        logged.push(["warn", message, fields]);
      }
    },
    error: () => {},
  };
  return { logger, logged, stderr };
}
