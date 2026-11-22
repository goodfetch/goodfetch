// A remote Daemon's dropped link (spec 606, ADR 0079): the Scripted engine,
// played as a Daemon over a WebSocket, drops the Client with no close, then
// plays the rest of its script to the next Client that connects. The Client
// reaches it again by itself, greets it with the same hello, resubscribes
// after the last event it read, and sends again what it was waiting for
// only when it is a read or an idempotent write; an Effect in flight raises
// `connection_lost`. A Scripted Daemon exits 0 only once every step played,
// so its exit says the Client sent exactly what the script holds after the
// drop, the resubscription's position and credits included.
import { Collie, ConnectionLostError, type Page } from "@goodfetch/collie";
import { failure, scriptedEngineEnv, startScriptedDaemon, test } from "@goodfetch/test-support";
import { describe, expect, vi } from "vitest";

/** The token both scripts' `hello` carries. */
const TOKEN = "remote-daemon-token";

describe("a Connection to a remote Daemon whose link drops", () => {
  test(
    "reaches the Daemon again and resubscribes after the last event it read, so its listener loses none",
    { tags: ["story:11-6"] },
    async ({ testEnv, processes }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/remote-drop-resubscribes.json",
        "websocket",
      );
      const env = {
        ...scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json"),
        GOODFETCH_URL: `ws://${daemon.address}`,
        GOODFETCH_TOKEN: TOKEN,
      };
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env });
      const run = await collie.start();
      const session = await run.newSession();
      const pages: Page[] = [];

      const subscription = session.on("page", (page) => {
        pages.push(page);
      });
      await vi.waitFor(
        () => {
          expect(pages).toHaveLength(2);
        },
        { timeout: 10_000 },
      );
      subscription.close();
      await run.finish();
      await collie.close();

      expect(await daemon.exit).toMatchObject({ code: 0 });
    },
  );

  test(
    "sends a read in flight again once it reached the Daemon again, and raises connection_lost for an Effect, never sent again",
    {
      tags: [
        "client_case/dropped-link-resends-reads",
        "daemon",
        "story:11-6",
        "story:11-32",
        "error:connection_lost",
      ],
    },
    async ({ testEnv, processes, clientCase }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/remote-drop-resends-a-read.json",
        "websocket",
      );
      const env = {
        ...scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json"),
        GOODFETCH_URL: `ws://${daemon.address}`,
        GOODFETCH_TOKEN: TOKEN,
      };
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env });
      const run = await collie.start();

      const effect = failure(run.newPage());
      const read = collie.call("run.list", {});
      const [lost, listed] = await Promise.all([effect, read]);
      await run.finish();
      await collie.close();

      const exit = await daemon.exit;

      await clientCase.check(
        "the read in flight is sent again and answered, and the Effect raises connection_lost",
        () =>
          expect({ lost: lost instanceof ConnectionLostError, error: lost, listed }).toEqual({
            lost: true,
            error: expect.objectContaining({
              code: "connection_lost",
              group: "unavailable",
              operation: "run.new_page",
            }),
            listed: { runs: [] },
          }),
      );
      await clientCase.check(
        "the Daemon played its script through, so the Effect was never sent again",
        () => expect(exit).toMatchObject({ code: 0 }),
      );
    },
  );
});
