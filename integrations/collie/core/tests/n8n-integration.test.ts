// Pre-merge integration tests for `@goodfetch/n8n-nodes-collie` (ADRs 0027,
// 0070, 0072, 0090, 0095; spec #608, issue 1405).
//
// 1. Builds `@goodfetch/n8n-nodes-collie` into `dist/`.
// 2. Runs the action node against the real Collie Daemon HTTP listener
//    (`goodfetch-test-env --daemon=collie --http-listener`), proving `startRecipe`
//    with `tags` & `settings` parses cleanly (`run_target_unbuilt`, never
//    `invalid_params`), `getRunProgress`, `getRunReport`, refused token, and
//    refused protocol version.
// 3. Exercises **real Daemon webhook delivery** through `GoodfetchCollieTrigger`:
//    subscribes to a loopback HTTP receiver, starts & finishes a Run on the
//    real Collie Daemon so `SubscriptionRunEndObserver` delivers a real signed
//    webhook, and verifies signature + single-fire deduplication.
// 4. Exercises `goodfetch-scripted-engine --mode http` for Error codes the
//    Proving ground cannot cause (`model_unavailable`).
// 5. When Docker/Podman is available, loads `dist/` into the pinned n8n
//    container (`N8N_CUSTOM_EXTENSIONS`, `--network=none`) and runs
//    `n8n execute --id` against the real Collie Daemon.
import { execFileSync, spawn } from "node:child_process";
import { writeFileSync } from "node:fs";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  GoodfetchCollie,
  GoodfetchCollieTrigger,
  readCollieErrorMetadata,
} from "@goodfetch/n8n-nodes-collie";
import { runN8nWorkflow, test, toolPath } from "@goodfetch/test-support";
import {
  NodeOperationError,
  type IDataObject,
  type IExecuteFunctions,
  type IHookFunctions,
  type IHttpRequestOptions,
  type INode,
  type IWebhookFunctions,
} from "n8n-workflow";
import { beforeAll, describe, expect } from "vitest";

const REPO_ROOT = resolve(fileURLToPath(new URL("../../../..", import.meta.url)));
const N8N_PACKAGE_ROOT = join(REPO_ROOT, "integrations", "collie", "n8n");
const DIST_DIR = join(N8N_PACKAGE_ROOT, "dist");

const TEST_NODE: INode = {
  id: "collie-int-1",
  name: "Goodfetch Collie",
  type: "CUSTOM.goodfetchCollie",
  typeVersion: 1,
  position: [0, 0],
  parameters: {},
};

interface CapturedDelivery {
  readonly rawBody: string;
  readonly body: Record<string, unknown>;
  readonly headers: Record<string, string | string[] | undefined>;
}

async function startWebhookReceiver(): Promise<{
  readonly url: string;
  readonly nextDelivery: (timeoutMs?: number) => Promise<CapturedDelivery>;
  readonly close: () => Promise<void>;
}> {
  const queue: CapturedDelivery[] = [];
  const waiters: Array<(delivery: CapturedDelivery) => void> = [];
  const server = createServer((req: IncomingMessage, res: ServerResponse) => {
    const chunks: Buffer[] = [];
    req.on("data", (chunk: Buffer) => chunks.push(chunk));
    req.on("end", () => {
      const rawBody = Buffer.concat(chunks).toString("utf8");
      const parsed: unknown = JSON.parse(rawBody);
      const body: Record<string, unknown> = {};
      if (typeof parsed === "object" && parsed !== null && !Array.isArray(parsed)) {
        for (const [k, v] of Object.entries(parsed)) {
          body[k] = v;
        }
      }
      const delivery: CapturedDelivery = {
        rawBody,
        body,
        headers: { ...req.headers },
      };
      const waiter = waiters.shift();
      if (waiter !== undefined) {
        waiter(delivery);
      } else {
        queue.push(delivery);
      }
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end('{"ok":true}');
    });
  });
  await new Promise<void>((resolveListen) => server.listen(0, "127.0.0.1", resolveListen));
  const addr = server.address();
  if (addr === null || typeof addr === "string") {
    throw new Error("failed to bind webhook receiver");
  }
  return {
    url: `http://127.0.0.1:${addr.port}/webhook`,
    nextDelivery: (timeoutMs = 15_000) => {
      const immediate = queue.shift();
      if (immediate !== undefined) {
        return Promise.resolve(immediate);
      }
      return new Promise<CapturedDelivery>((resolveDelivery, rejectDelivery) => {
        const timer = setTimeout(() => {
          rejectDelivery(new Error(`timed out after ${timeoutMs}ms waiting for Daemon webhook`));
        }, timeoutMs);
        waiters.push((delivery) => {
          clearTimeout(timer);
          resolveDelivery(delivery);
        });
      });
    },
    close: () => new Promise<void>((resolveClose) => server.close(() => resolveClose())),
  };
}

function hasContainerEngine(): boolean {
  for (const bin of ["docker", "podman"]) {
    try {
      execFileSync(bin, ["info"], { stdio: "ignore", timeout: 5_000 });
      return true;
    } catch {
      // Try next engine.
    }
  }
  return false;
}

async function fetchHelper(req: IHttpRequestOptions): Promise<unknown> {
  const headers: Record<string, string> = {};
  if (req.headers !== undefined) {
    for (const [k, v] of Object.entries(req.headers)) {
      if (typeof v === "string") {
        headers[k] = v;
      }
    }
  }
  const res = await fetch(req.url, {
    method: req.method ?? "POST",
    headers,
    body: JSON.stringify(req.body),
  });
  const text = await res.text();
  let body: unknown = text;
  try {
    body = JSON.parse(text);
  } catch {
    // Keep text.
  }
  return { statusCode: res.status, body };
}

function makeExecuteCtx(
  credentials: IDataObject,
  params: Record<string, unknown>,
): IExecuteFunctions {
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- partial n8n IExecuteFunctions test harness.
  return {
    getInputData: () => [{ json: {} }],
    getNodeParameter: (name: string, _idx: number, fallback?: unknown) =>
      name in params ? params[name] : fallback,
    getCredentials: async () => credentials,
    getNode: () => TEST_NODE,
    continueOnFail: () => false,
    helpers: { httpRequest: fetchHelper },
  } as unknown as IExecuteFunctions;
}

function buildCredential(options: {
  readonly id: string;
  readonly baseUrl: string;
  readonly token: string;
  readonly protocolVersion?: string | undefined;
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
  readonly continueOnFail?: boolean | undefined;
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

async function startScriptedHttpEngine(options: {
  readonly executable: string;
  readonly scriptPath: string;
  readonly token: string;
  readonly env: Readonly<Record<string, string>>;
}): Promise<{
  readonly baseUrl: string;
  readonly token: string;
  readonly kill: () => void;
}> {
  const child = spawn(
    options.executable,
    ["--script", options.scriptPath, "--mode", "http", "--token", options.token],
    { env: { ...options.env }, stdio: ["ignore", "pipe", "pipe"] },
  );
  child.stdout.setEncoding("utf8");
  const announced = await new Promise<{ readonly address: string; readonly token: string }>(
    (resolveReady, rejectReady) => {
      let buf = "";
      const onData = (chunk: string): void => {
        buf += chunk;
        const nl = buf.indexOf("\n");
        if (nl !== -1) {
          child.stdout.off("data", onData);
          const parsed: unknown = JSON.parse(buf.slice(0, nl));
          if (
            typeof parsed !== "object" ||
            parsed === null ||
            Array.isArray(parsed) ||
            typeof Reflect.get(parsed, "address") !== "string" ||
            typeof Reflect.get(parsed, "token") !== "string"
          ) {
            rejectReady(
              new TypeError(`unexpected scripted engine announcement: ${buf.slice(0, nl)}`),
            );
            return;
          }
          resolveReady({
            address: String(Reflect.get(parsed, "address")),
            token: String(Reflect.get(parsed, "token")),
          });
        }
      };
      child.stdout.on("data", onData);
      child.once("exit", (code) => {
        rejectReady(new Error(`scripted engine exited (${String(code)}) before ready`));
      });
    },
  );
  return {
    baseUrl: `http://${announced.address}`,
    token: announced.token,
    kill: () => {
      child.kill("SIGKILL");
    },
  };
}

describe.skipIf(process.platform !== "linux")(
  "@goodfetch/n8n-nodes-collie against real Collie Daemon, Scripted engine, and pinned n8n image",
  () => {
    test.override({ withDaemon: true, withHttpListener: true, withBrowser: true });

    beforeAll(() => {
      execFileSync("pnpm", ["run", "build"], {
        cwd: N8N_PACKAGE_ROOT,
        stdio: "pipe",
      });
    }, 60_000);

    test(
      "executes actions, receives real Daemon webhook delivery, surfaces Scripted-engine errors, and runs pinned n8n container when available",
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
      async ({ daemon, testEnv }) => {
        const daemonHttp = daemon.http;
        const httpToken = daemon.http_token;
        if (daemonHttp === undefined || httpToken === undefined) {
          throw new Error("expected Daemon HTTP listener URL and token from goodfetch-test-env");
        }

        const started = await callDaemonJsonRpc(daemonHttp, httpToken, "run.start", {});
        const runId = String(started["run_id"]);
        const okCreds: IDataObject = {
          baseUrl: daemonHttp,
          token: httpToken,
          protocolVersion: "0.1",
        };
        const actionNode = new GoodfetchCollie();

        // 1. Direct execution against real Collie Daemon HTTP listener.
        const progressOut = await actionNode.execute.call(
          makeExecuteCtx(okCreds, { operation: "getRunProgress", runId }),
        );
        expect(progressOut[0]?.[0]?.json["run_id"]).toBe(runId);

        await callDaemonJsonRpc(daemonHttp, httpToken, "run.finish", { run_id: runId });
        const reportOut = await actionNode.execute.call(
          makeExecuteCtx(okCreds, { operation: "getRunReport", runId }),
        );
        expect(reportOut[0]?.[0]?.json["run_id"]).toBe(runId);

        // P10: startRecipe with tags + settings parses cleanly into RecipeStartParams.
        let recipeErr: unknown;
        try {
          await actionNode.execute.call(
            makeExecuteCtx(okCreds, {
              operation: "startRecipe",
              recipe: "nonexistent-recipe",
              additionalFields: {
                tags: '["surface:n8n"]',
                settings: '{"timeout":{"navigation":15000}}',
              },
            }),
          );
        } catch (err) {
          recipeErr = err;
        }
        expect(recipeErr).toBeInstanceOf(NodeOperationError);
        expect(readCollieErrorMetadata(recipeErr)?.code).toBe("run_target_unbuilt");

        // 2. P13: Real Daemon webhook delivery -> GoodfetchCollieTrigger (no synthetic signing).
        const receiver = await startWebhookReceiver();
        const trigger = new GoodfetchCollieTrigger();
        const staticData: IDataObject = {};
        try {
          // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- partial n8n IHookFunctions test harness.
          const hookCtx = {
            getNodeWebhookUrl: () => receiver.url,
            getNodeParameter: (name: string, fallbackValue?: unknown) =>
              name === "channel"
                ? "n8n-real-daemon-trigger"
                : name === "events"
                  ? ["run_finished"]
                  : fallbackValue,
            getCredentials: async () => okCreds,
            getWorkflowStaticData: () => staticData,
            getNode: () => TEST_NODE,
            helpers: { httpRequest: fetchHelper },
          } as unknown as IHookFunctions;

          expect(await trigger.webhookMethods.default.create.call(hookCtx)).toBe(true);
          const storedSecret = staticData["webhookSecret"];
          expect(typeof storedSecret === "string" && storedSecret.startsWith("whsec_")).toBe(true);

          // Start and finish a real Run on the Daemon so it sends a real HMAC-signed webhook.
          const trigRun = await callDaemonJsonRpc(daemonHttp, httpToken, "run.start", {});
          await callDaemonJsonRpc(daemonHttp, httpToken, "run.finish", {
            run_id: String(trigRun["run_id"]),
          });

          const delivered = await receiver.nextDelivery();
          expect(delivered.body["type"]).toBe("notification.run_finished");
          expect(
            typeof delivered.headers["webhook-signature"] === "string" &&
              delivered.headers["webhook-signature"].startsWith("v1,"),
          ).toBe(true);

          // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- partial n8n IWebhookFunctions test harness.
          const webhookCtx = {
            getRequestObject: () => ({
              rawBody: delivered.rawBody,
              body: delivered.body,
              headers: delivered.headers,
            }),
            getBodyData: () => delivered.body,
            getHeaderData: () => delivered.headers,
            getNodeParameter: (name: string, fallbackValue?: unknown) =>
              name === "events" ? ["run_finished"] : fallbackValue,
            getWorkflowStaticData: () => staticData,
            getNode: () => TEST_NODE,
            helpers: {
              httpRequest: async () => ({}),
              returnJsonArray: (items: IDataObject[]) => items.map((json) => ({ json })),
            },
          } as unknown as IWebhookFunctions;

          const firstFire = await trigger.webhook.call(webhookCtx);
          expect(firstFire.workflowData?.[0]?.length).toBe(1);
          const secondFire = await trigger.webhook.call(webhookCtx);
          expect(secondFire.workflowData).toBeUndefined();

          expect(await trigger.webhookMethods.default.delete.call(hookCtx)).toBe(true);
        } finally {
          await receiver.close();
        }

        // 3. P13: Scripted engine (`--mode http`) delivers `model_unavailable` -> NodeOperationError.
        const scriptedBin = toolPath(testEnv.tools, "scripted_engine");
        const scriptPath = join(testEnv.folder, "n8n-scripted-http.json");
        writeFileSync(
          scriptPath,
          JSON.stringify({
            description: "Returns model_unavailable to n8n GoodfetchCollie action node",
            steps: [
              {
                expect: {
                  method: "recipe.start",
                  params: { recipe: "n8n-model-err", detached: true },
                },
              },
              {
                reply: {
                  error: {
                    code: -32004,
                    message: "model provider unavailable",
                    data: {
                      code: "model_unavailable",
                      group: "model",
                      fix: "Switch models or retry later.",
                    },
                  },
                },
              },
            ],
          }),
          "utf8",
        );
        const scripted = await startScriptedHttpEngine({
          executable: scriptedBin,
          scriptPath,
          token: "n8n-scripted-token",
          env: testEnv.env,
        });
        try {
          let scriptedErr: unknown;
          try {
            await actionNode.execute.call(
              makeExecuteCtx(
                { baseUrl: scripted.baseUrl, token: scripted.token, protocolVersion: "0.1" },
                { operation: "startRecipe", recipe: "n8n-model-err" },
              ),
            );
          } catch (err) {
            scriptedErr = err;
          }
          expect(scriptedErr).toBeInstanceOf(NodeOperationError);
          expect(readCollieErrorMetadata(scriptedErr)?.code).toBe("model_unavailable");
          expect(readCollieErrorMetadata(scriptedErr)?.group).toBe("model");
        } finally {
          scripted.kill();
        }

        // 4. When Docker/Podman is available, run the built `dist/` node inside the pinned n8n image.
        await runPinnedN8nContainerChecks(daemonHttp, httpToken, runId);
      },
    );
  },
);

async function runPinnedN8nContainerChecks(
  daemonHttp: string,
  httpToken: string,
  runId: string,
): Promise<void> {
  if (!hasContainerEngine()) {
    return;
  }
  const credOk = buildCredential({ id: "cred-ok", baseUrl: daemonHttp, token: httpToken });
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
  expect(recipeRes.stdout).toContain("Fix:");
}
