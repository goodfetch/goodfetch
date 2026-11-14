#!/usr/bin/env node
// The `goodfetch-collie` bin of `@goodfetch/collie` (ADR 0090), so
// `npx @goodfetch/collie` runs Collie's executable: it finds the executable
// as `Collie.connect()` does and runs it with the process's arguments, stdio
// and environment, ending with its exit code. It writes nothing itself
// except Goodfetch's error when it cannot run the executable, which it
// writes as the executable writes a failed command (ADR 0069): one
// `{ "error": … }` on stdout under `--json`, else the message, then `code`,
// `retryable` and `fix` on lines of their own on stderr; and it exits with
// the code's row of the exit-code table, else its group's.
import { spawn } from "node:child_process";
import {
  EngineNotFoundError,
  GoodfetchError,
  errorCodeExitCodes,
  errorGroupExitCodes,
} from "@goodfetch/collie-client";
import { locateEngine, type EngineLocation } from "@goodfetch/collie-client/runtime";
import { collie } from "./product.ts";

const args = process.argv.slice(2);
const env = Object.fromEntries(
  Object.entries(process.env).filter((entry): entry is [string, string] => entry[1] !== undefined),
);

/** Whether the command line asks for `--json` output: a flag before any `--`. */
function json(): boolean {
  const end = args.indexOf("--");
  return (end === -1 ? args : args.slice(0, end)).includes("--json");
}

/** Writes `error` as the executable writes a failed command, then exits with its exit code. */
function fail(error: GoodfetchError): void {
  const exit = errorCodeExitCodes[error.code] ?? errorGroupExitCodes[error.group];
  const fix = error.fix === undefined ? {} : { fix: error.fix };
  const [stream, text] = json()
    ? [
        process.stdout,
        JSON.stringify({
          error: {
            code: error.code,
            group: error.group,
            retryable: error.retryable,
            message: error.message,
            ...fix,
            ...error.details,
          },
        }),
      ]
    : [
        process.stderr,
        [
          error.message,
          `code: ${error.code}`,
          `retryable: ${String(error.retryable)}`,
          ...(error.fix === undefined ? [] : [`fix: ${error.fix}`]),
        ].join("\n"),
      ];
  // Exiting once the write is flushed keeps all of it on a pipe.
  stream.write(`${text}\n`, () => process.exit(exit));
}

function run(): void {
  let location: EngineLocation;
  try {
    location = locateEngine(env, collie);
  } catch (error) {
    if (!(error instanceof GoodfetchError)) {
      throw error;
    }
    fail(error);
    return;
  }
  const child = spawn(location.path, args, { stdio: "inherit", env });
  let spawned = true;
  child.once("error", (cause) => {
    spawned = false;
    fail(new EngineNotFoundError(cause.message, { fix: location.notFoundFix, cause }));
  });
  child.once("exit", (code, signal) => {
    if (!spawned) {
      return;
    }
    if (signal !== null) {
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code ?? 1);
  });
}

run();
