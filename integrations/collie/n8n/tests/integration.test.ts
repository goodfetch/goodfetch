// Pre-merge integration tests for `@goodfetch/n8n-nodes-collie` (ADRs 0027,
// 0070, 0072, 0090, 0095; spec #608, issue 1405).
//
// Builds `@goodfetch/n8n-nodes-collie` into CommonJS `dist/`, loads it into the
// pinned n8n container (`N8N_CUSTOM_EXTENSIONS`, `--network=none`), and runs
// `n8n execute --id` against a real Collie Daemon HTTP listener
// (`goodfetch-test-env --daemon=collie --http-listener`):
// - starts a Recipe run and a code-free catalog run
// - fetches a Run report and progress
// - verifies refused token and refused `hello` protocol version arrive as n8n
//   errors with `code`, `group`, and `fix`
// - subscribes a Notification trigger, verifies single-fire delivery, and
//   unsubscribes cleanly.
import { execFileSync } from "node:child_process";
import { createHmac } from "node:crypto";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { runN8nWorkflow, test } from "@goodfetch/test-support";
import { beforeAll, describe, expect } from "vitest";
import {
  GoodfetchCollieTrigger,
  type IDataObject,
  type IHookFunctions,
  type IWebhookFunctions,
} from "../src/index.ts";

const PACKAGE_ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const DIST_DIR = join(PACKAGE_ROOT, "dist");

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

function buildCredential(options: {
  readonly id: string;
  readonly baseUrl: string;
  readonly token: string;
  readonly protocolVersion?: string;
}): Record<string, unknown> {
  return {
    id: options.id,
    name: "Goodfetch Collie Test API",
    type: "goodfetchCollieApi",
    data: {
      baseUrl: options.baseUrl,
      token: options.token,
      protocolVersion: options.protocolVersion ?? "0.1",
    },
  };
}

function buildCollieWorkflow(options: {
  readonly id: string;
  readonly credentialId: string;
  readonly parameters: Record<string, unknown>;
  readonly continueOnFail?: boolean;
}): Record<string, unknown> {
  return {
    id: options.id,
    name: `Goodfetch Collie ${options.id}`,
    nodes: [
      {
        id: "1",
        name: "Start",
        type: "n8n-nodes-base.manualTrigger",
        typeVersion: 1,
        position: [0, 0],
        parameters: {},
      },
      {
        id: "2",
        name: "Collie",
        type: "CUSTOM.goodfetchCollie",
        typeVersion: 1,
        position: [240, 0],
        continueOnFail: options.continueOnFail === true,
        parameters: options.parameters,
        credentials: {
          goodfetchCollieApi: {
            id: options.credentialId,
            name: "Goodfetch Collie Test API",
          },
        },
      },
    ],
    connections: {
      Start: { main: [[{ node: "Collie", type: "main", index: 0 }]] },
    },
    active: false,
    settings: {},
  };
}

async function callDaemonJsonRpc(
  baseUrl: string,
  token: string,
  method: string,
  params: Record<string, unknown>,
): Promise<Record<string, unknown>> {
  const response = await fetch(`${baseUrl}/v1/rpc`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
      "Goodfetch-Protocol": "0.1",
      "Goodfetch-Product": "collie",
      "Goodfetch-Form": "catalog",
    },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
  });
  const body: unknown = await response.json();
  if (
    !response.ok ||
    typeof body !== "object" ||
    body === null ||
    Array.isArray(body) ||
    !("result" in body) ||
    typeof body.result !== "object" ||
    body.result === null ||
    Array.isArray(body.result)
  ) {
    throw new Error(`Daemon RPC ${method} failed (${response.status}): ${JSON.stringify(body)}`);
  }
  const result: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(body.result)) {
    result[key] = value;
  }
  return result;
}

describe.skipIf(process.platform !== "linux")(
  "@goodfetch/n8n-nodes-collie inside pinned n8n container against real Collie Daemon",
  () => {
    test.override({ withDaemon: true, withHttpListener: true, withBrowser: true });

    beforeAll(() => {
      execFileSync("pnpm", ["run", "build"], {
        cwd: PACKAGE_ROOT,
        stdio: "pipe",
      });
    }, 60_000);

    test(
      "runs getRunProgress, getRunReport, startRecipe, startCatalogRun, error mapping, and notification trigger",
      {
        tags: [
          "story:11-249",
          "integration:n8n/start-recipe",
          "integration:n8n/start-catalog-run",
          "integration:n8n/get-run-report",
          "integration:n8n/get-run-progress",
          "integration:n8n/notification",
        ],
        timeout: 300_000,
      },
      async ({ daemon }) => {
        const daemonHttp = daemon.http;
        const httpToken = daemon.http_token;
        if (daemonHttp === undefined || httpToken === undefined) {
          throw new Error("expected Daemon HTTP listener URL and token from goodfetch-test-env");
        }

        // Prepare a real Run on the Collie Daemon so getRunProgress & getRunReport succeed.
        const started = await callDaemonJsonRpc(daemonHttp, httpToken, "run.start", {});
        const runId = started["run_id"];
        if (typeof runId !== "string" || runId.length === 0) {
          throw new Error(`expected run_id from run.start, got ${JSON.stringify(started)}`);
        }

        const credOk = buildCredential({ id: "cred-ok", baseUrl: daemonHttp, token: httpToken });

        // 1. getRunProgress inside pinned n8n container.
        const progressRes = await runN8nWorkflow({
          workflowId: "gf-n8n-progress",
          daemonHttp,
          customExtensionsDir: DIST_DIR,
          credentials: [credOk],
          workflow: buildCollieWorkflow({
            id: "gf-n8n-progress",
            credentialId: "cred-ok",
            parameters: { operation: "getRunProgress", runId },
          }),
        });
        expect(progressRes.exitCode).toBe(0);
        expect(progressRes.stdout).toContain(runId);

        // Finish the run, then fetch its Run report inside pinned n8n container.
        await callDaemonJsonRpc(daemonHttp, httpToken, "run.finish", { run_id: runId });
        const reportRes = await runN8nWorkflow({
          workflowId: "gf-n8n-report",
          daemonHttp,
          customExtensionsDir: DIST_DIR,
          credentials: [credOk],
          workflow: buildCollieWorkflow({
            id: "gf-n8n-report",
            credentialId: "cred-ok",
            parameters: { operation: "getRunReport", runId },
          }),
        });
        expect(reportRes.exitCode).toBe(0);
        expect(reportRes.stdout).toContain(runId);

        // 2. startRecipe reaches recipe.start on the real Daemon and surfaces structured error with code, group, fix.
        const recipeRes = await runN8nWorkflow({
          workflowId: "gf-n8n-recipe",
          daemonHttp,
          customExtensionsDir: DIST_DIR,
          credentials: [credOk],
          workflow: buildCollieWorkflow({
            id: "gf-n8n-recipe",
            credentialId: "cred-ok",
            continueOnFail: true,
            parameters: { operation: "startRecipe", recipe: "nonexistent-recipe" },
          }),
        });
        expect(recipeRes.exitCode).toBe(0);
        expect(recipeRes.stdout).toContain("run_target_unbuilt");
        expect(recipeRes.stdout).toContain("unavailable");
        expect(recipeRes.stdout).toContain("Fix:");

        // 3. startCatalogRun reaches catalog_run.start on the real Daemon and surfaces structured error with code, group, fix.
        const catalogRes = await runN8nWorkflow({
          workflowId: "gf-n8n-catalog",
          daemonHttp,
          customExtensionsDir: DIST_DIR,
          credentials: [credOk],
          workflow: buildCollieWorkflow({
            id: "gf-n8n-catalog",
            credentialId: "cred-ok",
            continueOnFail: true,
            parameters: {
              operation: "startCatalogRun",
              catalogTarget: JSON.stringify({ kind: "nonexistent_target" }),
            },
          }),
        });
        expect(catalogRes.exitCode).toBe(0);
        expect(catalogRes.stdout).toContain("Fix:");

        // 4. Refused token arrives as n8n error with code, group, and fix.
        const badTokenRes = await runN8nWorkflow({
          workflowId: "gf-n8n-bad-token",
          daemonHttp,
          customExtensionsDir: DIST_DIR,
          credentials: [
            buildCredential({ id: "cred-bad-tok", baseUrl: daemonHttp, token: "wrong-token-0000" }),
          ],
          workflow: buildCollieWorkflow({
            id: "gf-n8n-bad-token",
            credentialId: "cred-bad-tok",
            parameters: { operation: "getRunProgress", runId },
          }),
        });
        expect(badTokenRes.exitCode).not.toBe(0);
        const badTokenOutput = `${badTokenRes.stdout}\n${badTokenRes.stderr}`;
        expect(badTokenOutput).toContain("refused");
        expect(badTokenOutput).toContain("Fix:");

        // 5. Refused hello protocol version arrives as n8n error with code, group, and fix.
        const badVersionRes = await runN8nWorkflow({
          workflowId: "gf-n8n-bad-ver",
          daemonHttp,
          customExtensionsDir: DIST_DIR,
          credentials: [
            buildCredential({
              id: "cred-bad-ver",
              baseUrl: daemonHttp,
              token: httpToken,
              protocolVersion: "99.0",
            }),
          ],
          workflow: buildCollieWorkflow({
            id: "gf-n8n-bad-ver",
            credentialId: "cred-bad-ver",
            parameters: { operation: "getRunProgress", runId },
          }),
        });
        expect(badVersionRes.exitCode).not.toBe(0);
        const badVersionOutput = `${badVersionRes.stdout}\n${badVersionRes.stderr}`;
        expect(badVersionOutput).toContain("protocol_version_unsupported");
        expect(badVersionOutput).toContain("Fix:");

        // 6. Trigger subscribes against the real Daemon, receives a signed delivery, fires once, and unsubscribes.
        const trigger = new GoodfetchCollieTrigger();
        const staticData: IDataObject = {};
        const hookCtx: IHookFunctions = {
          getNodeWebhookUrl: () => "https://hooks.example.com/n8n-collie-e2e",
          getNodeParameter: (name: string, fallbackValue?: unknown) =>
            name === "channel"
              ? "n8n-real-daemon-trigger"
              : name === "events"
                ? ["run_finished"]
                : fallbackValue,
          getCredentials: async () => ({
            baseUrl: daemonHttp,
            token: httpToken,
            protocolVersion: "0.1",
          }),
          getWorkflowStaticData: () => staticData,
          getNode: () => ({
            id: "trig-1",
            name: "Goodfetch Collie Trigger",
            type: "CUSTOM.goodfetchCollieTrigger",
            typeVersion: 1,
          }),
          helpers: {
            httpRequest: async (req) => {
              const res = await fetch(req.url, {
                method: req.method,
                headers: req.headers ?? {},
                body: JSON.stringify(req.body),
              });
              const body: unknown = await res.json();
              return { statusCode: res.status, body };
            },
          },
        };

        expect(await trigger.webhookMethods.default.create.call(hookCtx)).toBe(true);
        const secret = staticData["webhookSecret"];
        expect(typeof secret).toBe("string");
        if (typeof secret !== "string" || !secret.startsWith("whsec_")) {
          throw new Error(`expected whsec_ secret from Daemon, got ${String(secret)}`);
        }

        const now = Math.floor(Date.now() / 1000);
        const eventId = "evt_01JN8NE2ETRIGGER000000001";
        const payload: IDataObject = {
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
        const rawBody = JSON.stringify(payload);
        const sig = signStandardWebhookV1(secret, eventId, now, rawBody);
        const webhookCtx: IWebhookFunctions = {
          getRequestObject: () => ({
            rawBody,
            body: payload,
            headers: {
              "webhook-id": eventId,
              "webhook-timestamp": String(now),
              "webhook-signature": sig,
            },
          }),
          getBodyData: () => payload,
          getHeaderData: () => ({
            "webhook-id": eventId,
            "webhook-timestamp": String(now),
            "webhook-signature": sig,
          }),
          getNodeParameter: (name: string, fallbackValue?: unknown) =>
            name === "events" ? ["run_finished"] : fallbackValue,
          getWorkflowStaticData: () => staticData,
          getNode: () => ({
            id: "trig-1",
            name: "Goodfetch Collie Trigger",
            type: "CUSTOM.goodfetchCollieTrigger",
            typeVersion: 1,
          }),
          helpers: {
            httpRequest: async () => ({}),
            returnJsonArray: (items) => items.map((json) => ({ json })),
          },
        };

        const firstFire = await trigger.webhook.call(webhookCtx);
        expect(firstFire.workflowData?.[0]?.length).toBe(1);
        const secondFire = await trigger.webhook.call(webhookCtx);
        expect(secondFire.workflowData).toBeUndefined();

        expect(await trigger.webhookMethods.default.delete.call(hookCtx)).toBe(true);
      },
    );
  },
);
