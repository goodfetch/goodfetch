// What a served test or Journey variant asks of its test's
// Daemon outside the page: its executable's commands, in the test's home,
// and a call over its socket as an SDK makes one, after `hello`.
import { execFile } from "node:child_process";
import { createConnection } from "node:net";
import { createInterface } from "node:readline";
import { promisify } from "node:util";
import type { ServedDaemon } from "./served-daemon.ts";

const execute = promisify(execFile);

/** Runs the executable of the test's Daemon with `args` in its home, answering what it prints. */
export async function command(daemon: ServedDaemon, args: readonly string[]): Promise<string> {
  const { stdout } = await execute(daemon.executable, [...args], { env: { ...daemon.env } });
  return stdout;
}

/** Calls `method` with `params` on the Daemon over its socket, as an SDK does, after `hello`. */
export async function callDaemon(
  daemon: ServedDaemon,
  method: string,
  params: unknown,
): Promise<unknown> {
  const info: unknown = JSON.parse(await command(daemon, ["engine", "info", "--json"]));
  const version =
    typeof info === "object" && info !== null && "product_version" in info
      ? String(info.product_version)
      : "";
  const socket = createConnection(daemon.socket);
  const lines = createInterface({ input: socket })[Symbol.asyncIterator]();
  const send = (id: number, name: string, values: unknown): void => {
    socket.write(`${JSON.stringify({ jsonrpc: "2.0", id, method: name, params: values })}\n`);
  };
  const answer = async (id: number): Promise<unknown> => {
    for (;;) {
      const line = await lines.next();
      if (line.done === true) {
        throw new Error(`the Daemon closed before answering ${method}`);
      }
      const message: unknown = JSON.parse(line.value);
      if (typeof message === "object" && message !== null && "id" in message && message.id === id) {
        if ("error" in message) {
          throw new Error(`${method} failed: ${JSON.stringify(message.error)}`);
        }
        return "result" in message ? message.result : undefined;
      }
    }
  };
  try {
    send(0, "hello", {
      protocol_version: "0.1",
      surface: "sdk_typescript",
      token: daemon.token,
      needs: "collie",
      product_version: version,
    });
    await answer(0);
    send(1, method, params);
    return await answer(1);
  } finally {
    socket.end();
  }
}
