// The named ports a test's Daemon keeps across its restarts (#2035): nothing
// asking the OS for a port can get one, and no two tests share one.
import { readFileSync } from "node:fs";
import { createServer, type Server } from "node:net";
import { describe, expect, onTestFinished, test } from "vitest";
import { reserveNamedPort, type NamedPort } from "../src/named-port.ts";

/** A named port released when the test ends. */
async function reserved(start?: number): Promise<NamedPort> {
  const named = await reserveNamedPort(start);
  onTestFinished(() => named.release());
  return named;
}

/** Listens on `port` as another program does, until the test ends. */
async function held(port: number): Promise<Server> {
  const server = createServer();
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", resolve);
  });
  onTestFinished(
    () =>
      new Promise<void>((resolve) => {
        server.close(() => {
          resolve();
        });
      }),
  );
  return server;
}

/** The first port this OS gives out for port 0 or as a connection's source. */
function firstEphemeralPort(): number {
  if (process.platform !== "linux") {
    return 49_152;
  }
  const [first] = readFileSync("/proc/sys/net/ipv4/ip_local_port_range", "utf8").split(/\s+/);
  return Number(first);
}

describe("reserveNamedPort", () => {
  test("gives a port the OS gives no program asking for one, on a port browsers open", async () => {
    const { port } = await reserved();

    expect(port).toBeLessThan(firstEphemeralPort());
    expect(port).toBeGreaterThan(10_080);
  });

  test("leaves the port free for the Daemon to bind", async () => {
    const { port } = await reserved();

    await expect(held(port)).resolves.toBeDefined();
  });

  test("never gives two tests one port, even looking from the same place", async () => {
    const first = await reserved(7);
    const second = await reserved(7);

    expect(second.port).not.toBe(first.port);
  });

  test("gives no port another program listens on", async () => {
    const first = await reserved(11);
    await first.release();
    await held(first.port);

    const next = await reserved(11);

    expect(next.port).not.toBe(first.port);
  });

  test("gives a released port again", async () => {
    const first = await reserved(13);
    await first.release();

    const again = await reserved(13);

    expect(again.port).toBe(first.port);
  });
});
