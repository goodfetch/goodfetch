// The handler-less starts (ADRs 0036 and 0102), the `handlerless-starts`
// client conformance case: a Recipe's run and a code-free catalog run of
// each target kind, started with every start option on the Scripted engine,
// which checks every message the SDK sends; a Run ID resumes an interrupted
// Run, logged at info, or returns a finished one ended, logged at warning.
// As a Daemon, each Run is detached, so closing its value and the
// Connection sends no `run.abort`: the Daemon keeps the Run once its program
// is gone. As the Attached engine, closing a Run's value aborts it. A view of
// a Run opened by its Run ID closes without stopping it either. The last two
// tests send the tags past their bounds that no script may send, to an
// Attached engine and to the Daemon of Collie's executable of the same commit.
import { Collie } from "@goodfetch/collie";
import {
  failure,
  scriptedEngineEnv,
  startRecordingLogger,
  startScriptedDaemon,
  test,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** What the start that resumed and the one that found its Run ended log, with their fields. */
function startLines(resumed: string, ended: string): unknown[] {
  return [
    [
      "info",
      `The start resumed the Run \`${resumed}\` from its committed state.`,
      { runId: resumed, startedAs: "resumed" },
    ],
    [
      "warn",
      `The start found the Run \`${ended}\` ended: it is returned as it ended and takes no new work.`,
      { runId: ended, startedAs: "ended" },
    ],
  ];
}

describe("A handler-less start on the Scripted engine as a Daemon", () => {
  test(
    "starts a Recipe's run and each code-free target kind with every option, logs a resume and an ended Run, and leaves each detached Run running",
    {
      tags: [
        "client_case/handlerless-starts",
        "daemon",
        "story:11-131",
        "operation:recipe.start",
        "operation:catalog_run.start",
      ],
    },
    async ({ testEnv, processes, clientCase }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "run/detached-starts-from-a-daemon.json",
        "socket",
      );
      const { logger, logged } = startRecordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env: testEnv.env, engine: "daemon", logger });
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
      const resumed = await collie.startRecipe("shop-prices", {
        runId: "weekly-prices",
        detached: true,
      });
      const ended = await collie.startCatalogRun(
        { kind: "plan_replay", plan: "plans/checkout.json" },
        { runId: "checkout-done", detached: true },
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
      const view = await collie.openRun("checkout-replay");
      await view.close();
      await collie.close();
      const exit = await daemon.exit;

      await clientCase.check("each start answers its Run as the engine started it", () =>
        expect(
          [recipe, extract, resumed, ended, agent, replay].map((run) => [
            run.id(),
            run.startedAs(),
          ]),
        ).toEqual([
          ["nightly-prices", "new"],
          ["extract-products", "new"],
          ["weekly-prices", "resumed"],
          ["checkout-done", "ended"],
          ["cheapest-kettle", "new"],
          ["checkout-replay", "new"],
        ]),
      );
      await clientCase.check(
        "the resume is logged at info, the ended Run at warning, through the host's logger",
        () => expect(logged).toEqual(startLines("weekly-prices", "checkout-done")),
      );
      await clientCase.check("openRun opens a view by its Run ID with no message", () =>
        expect(view.id()).toBe("checkout-replay"),
      );
      await clientCase.check(
        "the Daemon heard every option and no run.abort, so it played its script whole",
        () => expect(exit).toMatchObject({ code: 0 }),
      );
    },
  );
});

describe("A handler-less start on the Scripted engine as the Attached engine", () => {
  test(
    "sends every option but detached, aborts its Run on close, and logs a resume and an ended Run",
    {
      tags: [
        "client_case/handlerless-starts",
        "attached",
        "story:11-131",
        "operation:recipe.start",
        "operation:catalog_run.start",
      ],
    },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/handlerless-starts.json");
      const { logger, logged, stderr } = startRecordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });

      const begun = await collie.startRecipe("shop-prices", {
        runId: "nightly-prices",
        tags: ["nightly", "prices"],
        limits: { requests: 500 },
        settings: { timeout: { wait: 5_000 } },
        recording: { mode: "replay", path: "recordings/shop.warc" },
        timeout: 3_600_000,
      });
      await begun.close();
      const resumed = await collie.startRecipe("shop-prices", { runId: "nightly-prices" });
      await resumed.finish();
      const ended = await collie.startCatalogRun(
        {
          kind: "extract",
          url: "https://shop.example/products",
          schema: { type: "object", properties: { productName: { type: "string" } } },
        },
        { runId: "extract-products" },
      );
      const view = await collie.openRun("checkout-replay");
      await view.close();
      await collie.close();

      await clientCase.check("each start answers its Run as the engine started it", () =>
        expect([begun, resumed, ended].map((run) => [run.id(), run.startedAs()])).toEqual([
          ["nightly-prices", "new"],
          ["nightly-prices", "resumed"],
          ["extract-products", "ended"],
        ]),
      );
      await clientCase.check(
        "the resume is logged at info, the ended Run at warning, through the host's logger",
        () => expect(logged).toEqual(startLines("nightly-prices", "extract-products")),
      );
      await clientCase.check("openRun opens a view by its Run ID with no message", () =>
        expect(view.id()).toBe("checkout-replay"),
      );
      await clientCase.check(
        "the script played to its end, its Run aborted on close, so its engine wrote no failure",
        () => expect(stderr).toEqual([]),
      );
    },
  );
});

/** What the engine `engine` names answers starts with too many tags, a tag too long, and tags at their bounds. */
async function refusedTags(
  env: Record<string, string>,
  engine: "attached" | "daemon",
): Promise<Record<"tooMany" | "tooLong" | "unbuilt", unknown>> {
  // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
  await using collie = await Collie.connect({ env, engine });
  const replay = { kind: "plan_replay", plan: "plans/checkout.json" } as const;
  const tooMany = await failure(
    collie.startRecipe("shop-prices", { tags: Array.from({ length: 17 }, () => "t") }),
  );
  const tooLong = await failure(collie.startCatalogRun(replay, { tags: ["t".repeat(65)] }));
  const unbuilt = await failure(
    collie.startCatalogRun(replay, { tags: Array.from({ length: 16 }, () => "t".repeat(64)) }),
  );
  return { tooMany, tooLong, unbuilt };
}

/** Tags past their bounds are refused, and tags at their bounds reach a start the engine runs no target of yet. */
const REFUSED = {
  tooMany: { code: "invalid_params" },
  tooLong: { code: "invalid_params" },
  unbuilt: { code: "run_target_unbuilt", operation: "catalog_run.start", target: "plan_replay" },
};

describe("A handler-less start on the engine of the same commit", () => {
  test.override("withDaemon", true);

  test(
    "refuses tags past their bounds as an Attached engine, and runs no Recipe or code-free target yet",
    { tags: ["client_case/handlerless-starts", "attached", "story:11-131"] },
    async ({ env, clientCase }) => {
      const refused = await refusedTags(env, "attached");

      await clientCase.check(
        "tags past their bounds are refused, and tags at them reach the start",
        () => expect(refused).toMatchObject(REFUSED),
      );
    },
  );

  test(
    "refuses tags past their bounds as the Daemon, and runs no Recipe or code-free target yet",
    { tags: ["client_case/handlerless-starts", "daemon", "story:11-131"] },
    async ({ env, clientCase }) => {
      const refused = await refusedTags(env, "daemon");

      await clientCase.check(
        "tags past their bounds are refused, and tags at them reach the start",
        () => expect(refused).toMatchObject(REFUSED),
      );
    },
  );
});
