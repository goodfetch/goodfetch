// Seam B (spec #325): the `daemon` fixture, used exactly as a product test
// uses it, against the real `goodfetch-test-env --daemon` and the commit's
// Collie or Foxhound executable, which the tool manifest names. A Client
// reaches the Daemon of the test's own home through the described socket
// with the described token.
import { readFileSync } from "node:fs";
import { connect, type Socket } from "node:net";
import { join } from "node:path";
import { describe, expect } from "vitest";
import { test, type DaemonDescription, type Product } from "@goodfetch/test-support";

/** The Protocol version and Product version of this commit's executables. */
const PROTOCOL_VERSION = "0.1";
const PRODUCT_VERSION = "0.0.0";

/** Opens the described socket: a Unix socket's path, or loopback TCP's `host:port` on Windows (ADR 0024). */
function open(socket: string): Socket {
  if (process.platform !== "win32") {
    return connect(socket);
  }
  const end = socket.lastIndexOf(":");
  return connect({ host: socket.slice(0, end), port: Number(socket.slice(end + 1)) });
}

/** The Daemon's answer to a `hello` with `token` needing `product`; `undefined` when it closes unanswered. */
function hello(daemon: DaemonDescription, token: string, product: Product): Promise<unknown> {
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

describe("daemon", () => {
  test.override("withDaemon", true);

  test("is Collie's Daemon, reached through its socket with its token", async ({ daemon }) => {
    const answer = await hello(daemon, daemon.token, "collie");

    expect(answer).toMatchObject({ result: { product: "collie" } });
  });

  test("refuses a hello without its token", async ({ daemon }) => {
    const answer = await hello(daemon, `${daemon.token}0`, "collie");

    expect(answer).toBeUndefined();
  });

  test("is the Daemon whose runtime file is in the test's own home", ({ daemon, testEnv }) => {
    const runtimeFile = JSON.parse(
      readFileSync(join(testEnv.home, "0.0", "daemon.json"), "utf8"),
    ) as unknown;

    expect(runtimeFile).toMatchObject({ token: daemon.token });
  });
});

describe("daemon of a test that needs Foxhound", () => {
  test.override({ withDaemon: true, product: "foxhound" });

  test("is run by Foxhound's executable", async ({ daemon }) => {
    const answer = await hello(daemon, daemon.token, "foxhound");

    expect(answer).toMatchObject({ result: { product: "foxhound" } });
  });
});

describe("daemon without withDaemon", () => {
  test.fails("fails the fixture", ({ daemon }) => {
    expect(daemon).toBeDefined();
  });
});
