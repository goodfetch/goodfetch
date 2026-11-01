// A port a test's Daemon keeps across its restarts (#2035). A port the OS chose
// for port 0 is free between the Daemon's end and its next start, and any program
// asking for port 0 can get it then, which fails the restart with
// `console_port_in_use`. No OS gives out a named port for port 0 or as a
// connection's source: each one's ephemeral range starts above them. So only a
// program naming the port can take it, and test support's claim keeps every
// other test off it: the claim is a listener of its own on a second port, which
// the OS refuses to a second holder and frees when its holder dies, however that
// happens. The port itself stays free, for the Daemon to bind.
import { createServer, type Server } from "node:net";
import { TestSupportError } from "./errors.ts";

/**
 * The first named port. Above every port browsers refuse to open (10080 is
 * the highest), below the service manager tests' named ports (from 20000) and
 * below every OS's ephemeral range (from 32768).
 */
const FIRST_PORT = 11_000;

/** How many named ports there are. */
const PORTS = 4_000;

/** How far above its port each port's claim is, past the last named port. */
const CLAIM_OFFSET = PORTS;

const LOOPBACK = "127.0.0.1";

/** A named port that is this test's until it releases it. */
export interface NamedPort {
  /** The port, free for the test's Daemon to bind. */
  readonly port: number;
  /** Lets another test take the port. */
  release(): Promise<void>;
}

/** A listener on `port`, or `undefined` when another program holds it. */
async function listening(port: number): Promise<Server | undefined> {
  const server = createServer();
  return new Promise((resolve, reject) => {
    server.once("error", (error: NodeJS.ErrnoException) => {
      if (error.code === "EADDRINUSE") {
        resolve(undefined);
      } else {
        reject(error);
      }
    });
    server.listen({ port, host: LOOPBACK, exclusive: true }, () => {
      // A claim never keeps its process alive.
      server.unref();
      resolve(server);
    });
  });
}

async function closed(server: Server): Promise<void> {
  await new Promise((resolve) => server.close(resolve));
}

/**
 * Claims the first named port free of every other test and program, looking
 * from the named port at `start` (taken modulo their number): the process's
 * ID by default, so that parallel workers mostly look in different places.
 *
 * @throws {TestSupportError} `named_ports_exhausted` when every one is taken.
 */
export async function reserveNamedPort(start: number = process.pid): Promise<NamedPort> {
  for (let tried = 0; tried < PORTS; tried += 1) {
    const port = FIRST_PORT + ((start + tried) % PORTS);
    const claim = await listening(port + CLAIM_OFFSET);
    if (claim !== undefined) {
      const free = await listening(port);
      if (free !== undefined) {
        await closed(free);
        return { port, release: async () => closed(claim) };
      }
      await closed(claim);
    }
  }
  throw new TestSupportError(
    "named_ports_exhausted",
    `no port of ${String(FIRST_PORT)} to ${String(FIRST_PORT + PORTS - 1)} is free for a test's Daemon`,
  );
}
