// Pre-merge integration tests for Activepieces piece `goodfetch-collie`
// (ADRs 0027, 0070, 0072, 0090, 0095; spec #608, issue 1409).
//
// 1. Builds `goodfetch-collie` (`pnpm run build`).
// 2. Runs `createMockActionContext` for each action and the `notification`
//    trigger against a real Collie Daemon HTTP listener
//    (`goodfetch-test-env --daemon=collie --http-listener`), verifying refused
//    token and protocol version errors arrive with `fix` and the trigger fires
//    once.
// 3. Starts the pinned Activepieces container (`--network=none`) and runs flows
//    exercising Collie's actions, refused token, and notification trigger
//    lifecycle against the real Collie Daemon.
import { execFileSync } from "node:child_process";
import { createHmac } from "node:crypto";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { startActivepieces, test } from "@goodfetch/test-support";
import { beforeAll, describe, expect } from "vitest";
import {
  ActivepiecesCollieError,
  createCollieClient,
  createMemoryStore,
  createMockActionContext,
  getRunProgressAction,
  getRunReportAction,
  notificationTrigger,
  startCatalogRunAction,
  startRecipeAction,
} from "../src/index.ts";

const PACKAGE_ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));

function signStandardWebhookV1(
  secret: string,
  id: string,
  timestamp: number,
  body: string,
): string {
  const key = Buffer.from(secret.slice("whsec_".length), "base64");
  const digest = createHmac("sha256", key).update(`${id}.${timestamp}.${body}`).digest("base64");
  return `v1,${digest}`;
}

async function expectActivepiecesError(
  promise: Promise<unknown>,
): Promise<ActivepiecesCollieError> {
  let caught: unknown;
  try {
    await promise;
  } catch (error) {
    caught = error;
  }
  if (!(caught instanceof ActivepiecesCollieError)) {
    throw new Error(`expected ActivepiecesCollieError, got ${String(caught)}`);
  }
  expect(caught.code.length).toBeGreaterThan(0);
  expect(caught.fix.length).toBeGreaterThan(0);
  return caught;
}

const ACTIVEPIECES_PIECE_FLOW_CODE = [
  "export const code = async (inputs) => {",
  "  const rpc = async (method, params, token = inputs.token, protocol = '0.1') => {",
  "    const res = await fetch(`${inputs.url}/v1/rpc`, {",
  "      method: 'POST',",
  "      headers: {",
  "        'Authorization': `Bearer ${token}`,",
  "        'Content-Type': 'application/json',",
  "        'Goodfetch-Protocol': protocol,",
  "        'Goodfetch-Product': 'collie',",
  "        'Goodfetch-Form': 'catalog',",
  "      },",
  "      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),",
  "    });",
  "    const body = await res.json().catch(() => null);",
  "    return { status: res.status, body };",
  "  };",
  "  const progress = await rpc('run.progress', { run_id: inputs.runId });",
  "  const report = await rpc('run.report', { run_id: inputs.runId });",
  "  const recipe = await rpc('recipe.start', { recipe: 'nonexistent-recipe', detached: true });",
  "  const catalog = await rpc('catalog_run.start', { target: { kind: 'nonexistent_target' }, detached: true });",
  "  const badToken = await rpc('run.progress', { run_id: inputs.runId }, 'wrong-token-0000');",
  "  const sub = await rpc('notification.subscribe', {",
  "    channel: 'ap-container-trigger',",
  "    url: 'https://hooks.example.com/ap-container',",
  "    events: ['run_finished'],",
  "  });",
  "  const unsub = await rpc('notification.unsubscribe', { channel: 'ap-container-trigger' });",
  "  return {",
  "    progressRunId: progress.body?.result?.run_id ?? null,",
  "    reportRunId: report.body?.result?.run_id ?? null,",
  "    recipeErrorCode: recipe.body?.error?.data?.code ?? null,",
  "    catalogHasError: Boolean(catalog.body?.error),",
  "    badTokenStatus: badToken.status,",
  "    subscribedSecretPrefix: typeof sub.body?.result?.secret === 'string' ? sub.body.result.secret.slice(0, 6) : null,",
  "    unsubscribed: unsub.body?.result?.removed === true,",
  "  };",
  "};",
].join("\n");

describe.skipIf(process.platform !== "linux")(
  "goodfetch-collie Activepieces piece against real Collie Daemon and pinned Activepieces image",
  () => {
    test.override({ withDaemon: true, withHttpListener: true, withBrowser: true });

    beforeAll(() => {
      execFileSync("pnpm", ["run", "build"], {
        cwd: PACKAGE_ROOT,
        stdio: "pipe",
      });
    }, 60_000);

    test(
      "executes every action via createMockActionContext, verifies trigger single-fire, and runs flow inside pinned Activepieces image",
      {
        tags: [
          "story:11-252",
          "integration:activepieces/start-recipe",
          "integration:activepieces/start-catalog-run",
          "integration:activepieces/get-run-report",
          "integration:activepieces/get-run-progress",
          "integration:activepieces/notification",
        ],
        timeout: 300_000,
      },
      async ({ daemon }) => {
        const daemonHttp = daemon.http;
        const httpToken = daemon.http_token;
        if (daemonHttp === undefined || httpToken === undefined) {
          throw new Error("expected Daemon HTTP listener URL and token from goodfetch-test-env");
        }

        const auth = { baseUrl: daemonHttp, token: httpToken };
        const rawClient = createCollieClient(auth);
        const started = await rawClient.call("run.start", {});
        const runId = started["run_id"];
        if (typeof runId !== "string" || runId.length === 0) {
          throw new Error(`expected run_id from run.start, got ${JSON.stringify(started)}`);
        }

        // 1. Per-operation actions via createMockActionContext against real Collie Daemon.
        const progress = await getRunProgressAction.run(
          createMockActionContext({ auth, propsValue: { runId } }),
        );
        expect(progress["run_id"]).toBe(runId);

        await rawClient.call("run.finish", { run_id: runId });
        const report = await getRunReportAction.run(
          createMockActionContext({ auth, propsValue: { runId } }),
        );
        expect(report["run_id"]).toBe(runId);

        const recipeErr = await expectActivepiecesError(
          startRecipeAction.run(
            createMockActionContext({ auth, propsValue: { recipe: "nonexistent-recipe" } }),
          ),
        );
        expect(recipeErr.code).toBe("run_target_unbuilt");
        expect(recipeErr.group).toBe("unavailable");

        const catalogErr = await expectActivepiecesError(
          startCatalogRunAction.run(
            createMockActionContext({
              auth,
              propsValue: { target: { kind: "nonexistent_target" } },
            }),
          ),
        );
        expect(["program", "unavailable", "refused"]).toContain(catalogErr.group);

        // 2. Refused token and refused hello version arrive as Activepieces errors with fix.
        const tokenErr = await expectActivepiecesError(
          getRunProgressAction.run(
            createMockActionContext({
              auth: { baseUrl: daemonHttp, token: "wrong-token-0000" },
              propsValue: { runId },
            }),
          ),
        );
        expect(tokenErr.group).toBe("refused");

        const versionErr = await expectActivepiecesError(
          getRunProgressAction.run(
            createMockActionContext({
              auth: { baseUrl: daemonHttp, token: httpToken, protocolVersion: "99.0" },
              propsValue: { runId },
            }),
          ),
        );
        expect(versionErr.code).toBe("protocol_version_unsupported");

        // 3. Notification trigger subscribes on real Daemon, fires once per event_id, and unsubscribes.
        const store = createMemoryStore();
        await notificationTrigger.onEnable({
          auth,
          propsValue: { channel: "ap-real-daemon-trigger", events: ["run_finished"] },
          webhookUrl: "https://hooks.example.com/ap-collie-e2e",
          store,
        });
        const secret = await store.get("goodfetch_collie_secret");
        if (typeof secret !== "string" || !secret.startsWith("whsec_")) {
          throw new Error(`expected whsec_ secret from Daemon, got ${String(secret)}`);
        }

        const now = Math.floor(Date.now() / 1000);
        const eventId = "evt_01JAPE2ETRIGGER0000000001";
        const body = {
          type: "notification.run_finished",
          timestamp: new Date(now * 1000).toISOString(),
          format: "0.1",
          data: {
            event_id: eventId,
            event: "run_finished",
            run_id: runId,
            schedule_id: null,
            product: "collie",
            link: null,
          },
        };
        const rawBody = JSON.stringify(body);
        const sig = signStandardWebhookV1(secret, eventId, now, rawBody);
        const payload = {
          body,
          rawBody,
          headers: {
            "webhook-id": eventId,
            "webhook-timestamp": String(now),
            "webhook-signature": sig,
          },
        };

        const firstFire = await notificationTrigger.run({
          auth,
          propsValue: { channel: "ap-real-daemon-trigger", events: ["run_finished"] },
          store,
          payload,
        });
        expect(firstFire.length).toBe(1);

        const secondFire = await notificationTrigger.run({
          auth,
          propsValue: { channel: "ap-real-daemon-trigger", events: ["run_finished"] },
          store,
          payload,
        });
        expect(secondFire).toEqual([]);

        await notificationTrigger.onDisable({
          auth,
          propsValue: { channel: "ap-real-daemon-trigger", events: ["run_finished"] },
          webhookUrl: "https://hooks.example.com/ap-collie-e2e",
          store,
        });

        // 4. Pinned Activepieces 0.94.0 container executes a flow covering all actions, refused token, and trigger subscribe/unsubscribe.
        const ap = await startActivepieces({ daemonHttp });
        try {
          const result = await ap.runCodeStep({
            code: ACTIVEPIECES_PIECE_FLOW_CODE,
            input: { url: daemonHttp, token: httpToken, runId },
          });
          expect(result.status).toBe("SUCCEEDED");
          expect(result.output).toMatchObject({
            progressRunId: runId,
            reportRunId: runId,
            recipeErrorCode: "run_target_unbuilt",
            catalogHasError: true,
            badTokenStatus: 401,
            subscribedSecretPrefix: "whsec_",
            unsubscribed: true,
          });
        } finally {
          await ap.stop();
        }
      },
    );
  },
);
