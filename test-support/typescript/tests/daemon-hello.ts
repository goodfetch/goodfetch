// A Client's `hello` to a Daemon `goodfetch-test-env --daemon` started, as
// the tests of test support's `daemon` fixtures send it (ADR 0024): one
// JSON-RPC line on the described socket, carrying the described token.
import { readFileSync } from "node:fs";
import { connect, type Socket } from "node:net";
import type { DaemonDescription, Product } from "@goodfetch/test-support";

/** The Protocol version of this commit's executables. */
const PROTOCOL_VERSION = "0.1";
/** Their Product version: the workspace's, which every executable reports. */
const PRODUCT_VERSION = /\[workspace\.package\]\nversion = "([^"]+)"/.exec(
  readFileSync(new URL("../../../Cargo.toml", import.meta.url), "utf8"),
)?.[1];

/** Opens the described socket: a Unix socket's path, or loopback TCP's `host:port` on Windows (ADR 0024). */
function open(socket: string): Socket {
  if (process.platform !== "win32") {
    return connect(socket);
  }
  const end = socket.lastIndexOf(":");
  return connect({ host: socket.slice(0, end), port: Number(socket.slice(end + 1)) });
}

/** The Daemon's answer to a `hello` with `token` needing `product`; `undefined` when it closes unanswered. */
export function hello(
  daemon: Pick<DaemonDescription, "socket">,
  token: string,
  product: Product,
): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const socket = open(daemon.socket);
    let text = "";
    socket.setEncoding("utf8");
    socket.on("connect", () => {
      const request = {
        jsonrpc: "2.0",
        id: 0,
        method: "hello",
        params: {
          protocol_version: PROTOCOL_VERSION,
          surface: "sdk_typescript",
          token,
          needs: product,
          product_version: PRODUCT_VERSION,
        },
      };
      socket.write(`${JSON.stringify(request)}\n`);
    });
    socket.on("data", (chunk: string) => {
      text += chunk;
      const end = text.indexOf("\n");
      if (end !== -1) {
        socket.destroy();
        resolve(JSON.parse(text.slice(0, end)));
      }
    });
    socket.on("close", () => {
      resolve(undefined);
    });
    socket.on("error", reject);
  });
}
