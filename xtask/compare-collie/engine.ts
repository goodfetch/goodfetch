// Collie's Attached engine for the Collie scenario's two Collie rows (issue
// 2406): started by the driver, as a program starts it, from the path in
// the environment xtask set, with the driver's whole environment. It stays
// in the driver's process tree, so the tree's CPU time is the program's and
// the engine's; its stderr is the driver's, which xtask keeps. Its stdin
// counts the JSON-RPC messages the driver sends it, one per line.
import { spawn } from "node:child_process";
import process from "node:process";
import { Readable, Writable } from "node:stream";

import {
  ATTACHED_ENGINE_COMMAND,
  COLLIE_ENGINE_PATH_VARIABLE,
} from "@goodfetch/collie-client/generated";

/** A running engine. */
export interface Engine {
  /** Its stdio, a Connection's transport. */
  readonly transport: ReadableWritablePair<Uint8Array, Uint8Array>;
  /** The JSON-RPC messages sent it so far. */
  sent(): number;
  /** Settles once its process has ended. */
  readonly exited: Promise<void>;
}

const NEWLINE = 0x0a;

/** Starts the engine whose path xtask set. */
export async function startEngine(): Promise<Engine> {
  const path = process.env[COLLIE_ENGINE_PATH_VARIABLE];
  if (path === undefined || path === "") {
    throw new Error(`${COLLIE_ENGINE_PATH_VARIABLE} names no engine`);
  }
  const child = spawn(path, [ATTACHED_ENGINE_COMMAND], {
    env: process.env,
    stdio: ["pipe", "pipe", "inherit"],
  });
  await new Promise<void>((resolve, reject) => {
    child.once("spawn", resolve);
    child.once("error", reject);
  });
  const { stdin, stdout } = child;
  if (stdin === null || stdout === null) {
    throw new TypeError("invariant: the engine's stdin and stdout are piped");
  }
  // A write after the engine ended fails the writer, which its user hears.
  stdin.on("error", () => {});
  const exited = new Promise<void>((resolve) => {
    child.once("close", () => resolve());
  });
  let sent = 0;
  const writer = (Writable.toWeb(stdin) as WritableStream<Uint8Array>).getWriter();
  const writable = new WritableStream<Uint8Array>({
    async write(chunk) {
      for (let at = chunk.indexOf(NEWLINE); at !== -1; at = chunk.indexOf(NEWLINE, at + 1)) {
        sent += 1;
      }
      await writer.write(chunk);
    },
    close: () => writer.close(),
    abort: (reason) => writer.abort(reason),
  });
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a stream with no encoding set reads Buffers, which are Uint8Arrays.
  const readable = Readable.toWeb(stdout) as ReadableStream<Uint8Array>;
  return {
    transport: { readable, writable },
    sent: () => sent,
    exited,
  };
}
