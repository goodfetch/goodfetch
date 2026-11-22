// A remote Daemon's dropped link under a Dataset's reader (spec 606): the
// Scripted engine, played as a Daemon over a WebSocket, drops the Client
// while `run.records` streams, then plays the rest of its script to the next
// Client that connects. The Client reaches it again by itself and sends the
// stream again after the last Record it read, so the reader gets each Record
// once. A Scripted Daemon exits 0 only once every step played, so its exit
// says the Client sent exactly what the script holds after the drop, the
// stream's `after` and credits included.
import { Foxhound, seed, type DatasetRecord } from "@goodfetch/foxhound";
import { scriptedEngineEnv, startScriptedDaemon, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { crawl, listing, products } from "./programs/two-page-types.ts";

/** The token the script's `hello` carries. */
const TOKEN = "remote-daemon-token";

describe("a Foxhound Connection to a remote Daemon whose link drops", () => {
  test(
    "sends a Dataset's stream of Records again after the last Record read, so its reader gets each once",
    { tags: ["story:11-6"] },
    async ({ testEnv, processes }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/remote-drop-resumes-records.json",
        "websocket",
      );
      const env = {
        ...scriptedEngineEnv(testEnv, "foxhound", "connect/foxhound-accepted.json"),
        GOODFETCH_URL: `ws://${daemon.address}`,
        GOODFETCH_TOKEN: TOKEN,
      };
      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const foxhound = await Foxhound.connect({ env });
      const run = await foxhound.startCrawl(crawl, {
        seeds: [seed("https://shop.test/a", listing)],
        runId: "nightly",
      });

      const records: DatasetRecord[] = [];
      for await (const record of run.records(products)) records.push(record);
      await run.finish();
      await foxhound.close();

      expect({
        numbers: records.map((record) => record.number),
        exit: await daemon.exit,
      }).toEqual({ numbers: [1, 2], exit: expect.objectContaining({ code: 0 }) });
    },
  );
});
