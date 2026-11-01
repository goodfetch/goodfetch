// The check the served-Daemon isolation test makes of a stopped Daemon's
// Console port (#1777), against a real Daemon and its own Console: its own
// listener answers as itself while it serves, and once it has stopped,
// neither the refused port nor another program's listener on the freed
// port does, even one that answers `GET /session` with a CSRF token.
import { createServer, type Server } from "node:http";
import { describe, expect, onTestFinished } from "vitest";
import { test as base } from "@goodfetch/test-support";
import { answersAs, consoleIdentity } from "../src/console-identity.ts";
import { reserveNamedPort } from "../src/named-port.ts";
import { executableConsole, startServedDaemon, type ServedDaemon } from "../src/served-daemon.ts";

/** An HTTP listener on `port` that answers every request `200` with a CSRF token of its own. */
async function foreignListener(port: number): Promise<Server> {
  const server = createServer((_, response) => {
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({ csrf_token: "another listener's", offline_cache: true }));
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", resolve);
  });
  return server;
}

/** A started Daemon, and what stops it. */
interface Served {
  readonly daemon: ServedDaemon;
  readonly close: () => Promise<void>;
}

/**
 * `started`: a served Daemon of Collie's executable serving its own
 * Console, stopped before the test's process record checks that nothing it
 * started is alive; its `close` stops it sooner, once.
 */
const test = base.extend<{ started: Served }>({
  started: async ({ processes, task, toolManifest }, use) => {
    // Named and claimed past the Daemon's stop, so the freed port waits for the test's own listener.
    const named = await reserveNamedPort();
    const started = await startServedDaemon({
      processes,
      testName: task.fullName,
      toolManifest,
      product: "collie",
      provingGroundSeed: undefined,
      consoleServer: executableConsole,
      consolePort: named.port,
      oidc: {
        issuer: "http://127.0.0.1:1",
        clientId: "console",
        clientSecret: "unused",
        redirectPath: "/sso/callback",
      },
    });
    let closed: Promise<void> | undefined;
    const close = async (): Promise<void> => {
      closed ??= started.close();
      await closed;
    };
    await use({ daemon: started.daemon, close });
    await close();
    await named.release();
  },
});

describe("answersAs", () => {
  test("is true while the Daemon that gave the identity serves its Console", async ({
    started: { daemon },
  }) => {
    const identity = await consoleIdentity(daemon);

    const answered = await answersAs(identity);

    expect(answered).toBe(true);
  });

  test("is false once its Daemon stopped, the port refused or another listener's", async ({
    started: { daemon, close },
  }) => {
    const identity = await consoleIdentity(daemon);
    await close();
    const refused = await answersAs(identity);
    const foreign = await foreignListener(daemon.port);
    onTestFinished(async () => {
      await new Promise((resolve) => foreign.close(resolve));
    });

    const anotherListeners = await answersAs(identity);

    expect({ refused, anotherListeners }).toEqual({ refused: false, anotherListeners: false });
  });
});
