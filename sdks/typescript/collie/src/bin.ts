#!/usr/bin/env node
// The `goodfetch-collie` bin of `@goodfetch/collie` (ADR 0090), so
// `npx @goodfetch/collie` runs Collie's executable: it finds the executable
// as `Collie.connect()` does and runs it with the process's arguments, stdio
// and environment, ending with its exit code. It writes nothing itself
// except Goodfetch's error when no executable is installed.
import { spawn } from "node:child_process";
import { GoodfetchError } from "@goodfetch/collie-client";
import { locateEngine } from "./engine-path.ts";

const env = Object.fromEntries(
  Object.entries(process.env).filter((entry): entry is [string, string] => entry[1] !== undefined),
);

let path: string;
try {
  path = locateEngine(env).path;
} catch (error) {
  if (!(error instanceof GoodfetchError)) {
    throw error;
  }
  process.stderr.write(`error[${error.code}]: ${error.message}\n${error.fix ?? ""}\n`);
  process.exit(1);
}

const child = spawn(path, process.argv.slice(2), { stdio: "inherit", env });
child.once("error", (error) => {
  process.stderr.write(`error[engine_not_found]: ${error.message}\n`);
  process.exit(1);
});
child.once("exit", (code, signal) => {
  if (signal !== null) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 1);
});
