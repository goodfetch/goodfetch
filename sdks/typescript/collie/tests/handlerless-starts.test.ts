// The handler-less starts (ADRs 0036 and 0102): a Recipe's run and a
// code-free catalog run of each target kind, started with every start
// option on the Scripted engine as a Daemon, which checks every message the
// SDK sends. Each Run is detached, so closing its value and the Connection
// sends no `run.abort`: the Daemon keeps the Run once its program is gone.
import { Collie } from "@goodfetch/collie";
import { startScriptedDaemon, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

describe("A handler-less start on the Scripted engine as a Daemon", () => {
  test(
    "starts a Recipe's run and each code-free target kind with every option, and leaves each detached Run running",
    { tags: ["daemon", "operation:recipe.start", "operation:catalog_run.start"] },
    async ({ testEnv, processes }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "run/detached-starts-from-a-daemon.json",
        "socket",
      );
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env: testEnv.env, engine: "daemon" });
      const recipe = await collie.startRecipe("shop-prices", {
        runId: "nightly-prices",
        tags: ["nightly", "prices"],
        limits: { requests: 500 },
        settings: { timeout: { wait: 5_000 } },
        recording: { mode: "replay", path: "recordings/shop.warc" },
        timeout: 3_600_000,
        detached: true,
      });
      const extract = await collie.startCatalogRun(
        {
          kind: "extract",
          url: "https://shop.example/products",
          schema: { type: "object", properties: { productName: { type: "string" } } },
          instruction: "Read each product's name.",
        },
        { runId: "extract-products", detached: true },
      );
      const agent = await collie.startCatalogRun(
        {
          kind: "agent",
          url: "https://shop.example/",
          task: "Find the cheapest kettle.",
          expect: { found: true },
          schema: { type: "object", properties: { unitPrice: { type: "number" } } },
          maxSteps: 20,
        },
        { runId: "cheapest-kettle", timeout: null, detached: true },
      );
      const replay = await collie.startCatalogRun(
        { kind: "plan_replay", plan: "plans/checkout.json" },
        {
          runId: "checkout-replay",
          recording: { mode: "record", path: "recordings/checkout.warc" },
          detached: true,
        },
      );
      await recipe.close();
      await collie.close();
      const exit = await daemon.exit;

      expect({
        started: [recipe, extract, agent, replay].map((run) => [run.id(), run.startedAs()]),
        exit,
      }).toMatchObject({
        started: [
          ["nightly-prices", "new"],
          ["extract-products", "new"],
          ["cheapest-kettle", "new"],
          ["checkout-replay", "new"],
        ],
        exit: { code: 0 },
      });
    },
  );
});
