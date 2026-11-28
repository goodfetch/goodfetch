// Unit tests for `@goodfetch/n8n-nodes-collie` (ADRs 0027, 0070, 0072, 0090,
// 0095; spec #608, issue 1405): official strict `n8n-node lint`, complete
// Collie catalog Error code mapping, P10 `RecipeStartParams` alignment,
// refused token and protocol version errors, S2 fail-closed webhook trigger
// verification + idempotent re-subscribe + single-fire dedup, and `n8n-scanner`.
import { execFileSync } from "node:child_process";
import { createHmac } from "node:crypto";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  GoodfetchCollie,
  GoodfetchCollieApi,
  GoodfetchCollieTrigger,
  mapCatalogErrorToN8n,
  readCollieErrorMetadata,
} from "@goodfetch/n8n-nodes-collie";
import {
  NodeOperationError,
  type IDataObject,
  type IExecuteFunctions,
  type IHookFunctions,
  type IHttpRequestOptions,
  type INode,
  type IWebhookFunctions,
} from "n8n-workflow";
import { describe, expect, test } from "vitest";
import { runN8nScanner, validatePublishedManifest } from "../../../../ci/live/n8n-scanner.ts";

const REPO_ROOT = resolve(fileURLToPath(new URL("../../../..", import.meta.url)));
const N8N_PACKAGE_ROOT = join(REPO_ROOT, "integrations", "collie", "n8n");

const TEST_NODE: INode = {
  id: "collie-1",
  name: "Goodfetch Collie",
  type: "CUSTOM.goodfetchCollie",
  typeVersion: 1,
  position: [0, 0],
  parameters: {},
};

function readJson(path: string): Record<string, unknown> {
  const parsed: unknown = JSON.parse(readFileSync(path, "utf8"));
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new TypeError(`expected JSON object at ${path}`);
  }
  const result: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(parsed)) {
    result[key] = value;
  }
  return result;
}

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

function expectRecord(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new TypeError(`expected object for ${label}`);
  }
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(value)) {
    out[k] = v;
  }
  return out;
}

function makeExecuteContext(options: {
  readonly params: Record<string, unknown>;
  readonly credentials?: IDataObject | undefined;
  readonly httpRequest: (req: IHttpRequestOptions) => Promise<unknown>;
  readonly continueOnFail?: boolean | undefined;
}): IExecuteFunctions {
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- partial n8n IExecuteFunctions test harness.
  return {
    getInputData: () => [{ json: {} }],
    getNodeParameter: (name: string, _index: number, fallbackValue?: unknown) =>
      name in options.params ? options.params[name] : fallbackValue,
    getCredentials: async () =>
      options.credentials ?? {
        baseUrl: "http://127.0.0.1:7801",
        token: "test-token-0000",
        protocolVersion: "0.1",
      },
    getNode: () => TEST_NODE,
    continueOnFail: () => options.continueOnFail === true,
    helpers: {
      httpRequest: options.httpRequest,
    },
  } as unknown as IExecuteFunctions;
}

describe("@goodfetch/n8n-nodes-collie", () => {
  test(
    "passes official strict n8n-node lint (zero runtime deps, verified-node cloud rules)",
    { tags: ["story:11-249"] },
    () => {
      // `eslint-plugin-n8n-nodes-base` mocks filenames when `NODE_ENV=test` (Vitest).
      const lintEnv: NodeJS.ProcessEnv = { ...process.env };
      delete lintEnv["NODE_ENV"];
      execFileSync("pnpm", ["--dir", N8N_PACKAGE_ROOT, "run", "lint"], {
        stdio: "pipe",
        timeout: 60_000,
        env: lintEnv,
      });

      const pkg = readJson(join(N8N_PACKAGE_ROOT, "package.json"));
      expect(pkg["dependencies"]).toBeUndefined();

      const creds = new GoodfetchCollieApi();
      expect(creds.name).toBe("goodfetchCollieApi");
      expect(creds.authenticate.properties.headers?.["Goodfetch-Product"]).toBe("collie");
      expect(creds.authenticate.properties.headers?.["Goodfetch-Form"]).toBe("catalog");

      const node = new GoodfetchCollie();
      expect(node.description.name).toBe("goodfetchCollie");
      expect(node.description.usableAsTool).toBe(true);

      const trigger = new GoodfetchCollieTrigger();
      expect(trigger.description.name).toBe("goodfetchCollieTrigger");
    },
  );

  test(
    "maps every Error code in Collie's catalog to a NodeOperationError with code, group, and fix",
    { tags: ["story:11-249"] },
    () => {
      const catalog = readJson(
        join(REPO_ROOT, "crates", "goodfetch-protocol", "generated", "collie.json"),
      );
      const rawErrors = catalog["error_codes"];
      expect(Array.isArray(rawErrors)).toBe(true);
      if (!Array.isArray(rawErrors)) {
        throw new Error("expected error_codes array in collie.json");
      }
      expect(rawErrors.length).toBeGreaterThan(50);

      for (const raw of rawErrors) {
        if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
          throw new TypeError("expected error code object");
        }
        const name = Reflect.get(raw, "name");
        const group = Reflect.get(raw, "group");
        const description = Reflect.get(raw, "description");
        if (typeof name !== "string" || typeof group !== "string") {
          throw new TypeError("missing name or group in error_codes entry");
        }
        const mapped = mapCatalogErrorToN8n(
          {
            code: name,
            group,
            description: typeof description === "string" ? description : undefined,
          },
          TEST_NODE,
        );
        expect(mapped).toBeInstanceOf(NodeOperationError);
        const meta = readCollieErrorMetadata(mapped);
        expect(meta?.code).toBe(name);
        expect(meta?.group).toBe(group);
        expect((meta?.fix ?? "").length).toBeGreaterThan(0);
      }
    },
  );

  test(
    "executes all four actions without sending unknown recipe.start fields and surfaces refused errors with fix",
    {
      tags: [
        "story:11-249",
        "integration:n8n/start-recipe",
        "integration:n8n/start-catalog-run",
        "integration:n8n/get-run-report",
        "integration:n8n/get-run-progress",
      ],
    },
    async () => {
      const node = new GoodfetchCollie();
      const seenRequests: Array<{
        readonly method: string;
        readonly params: Record<string, unknown>;
      }> = [];

      const okCtx = (params: Record<string, unknown>) =>
        makeExecuteContext({
          params,
          httpRequest: async (req) => {
            expect(req.headers?.["Goodfetch-Product"]).toBe("collie");
            expect(req.headers?.["Goodfetch-Form"]).toBe("catalog");
            expect(req.headers?.["Authorization"]).toBe("Bearer test-token-0000");
            const body = expectRecord(req.body, "request body");
            const method = typeof body["method"] === "string" ? body["method"] : "";
            seenRequests.push({ method, params: expectRecord(body["params"], "request params") });
            return {
              statusCode: 200,
              body: { jsonrpc: "2.0", id: 1, result: { ok: true, method } },
            };
          },
        });

      const r1 = await node.execute.call(
        okCtx({
          operation: "startRecipe",
          recipe: "checkout",
          additionalFields: {
            tags: '["env:test"]',
            settings: '{"timeout":{"navigation":15000}}',
          },
        }),
      );
      const r2 = await node.execute.call(
        okCtx({
          operation: "startCatalogRun",
          catalogTarget: '{"kind":"extract","url":"https://example.com","schema":{}}',
        }),
      );
      const r3 = await node.execute.call(okCtx({ operation: "getRunReport", runId: "r_1" }));
      const r4 = await node.execute.call(okCtx({ operation: "getRunProgress", runId: "r_1" }));

      expect(seenRequests.map((r) => r.method)).toEqual([
        "recipe.start",
        "catalog_run.start",
        "run.report",
        "run.progress",
      ]);
      // P10: `recipe.start` must match `RecipeStartParams` (no unknown `params` field).
      expect(seenRequests[0]?.params["params"]).toBeUndefined();
      expect(seenRequests[0]?.params["recipe"]).toBe("checkout");
      expect(seenRequests[0]?.params["detached"]).toBe(true);
      expect(seenRequests[0]?.params["tags"]).toEqual(["env:test"]);
      expect(seenRequests[0]?.params["settings"]).toEqual({ timeout: { navigation: 15000 } });

      expect(r1[0]?.[0]?.json["method"]).toBe("recipe.start");
      expect(r2[0]?.[0]?.json["method"]).toBe("catalog_run.start");
      expect(r3[0]?.[0]?.json["method"]).toBe("run.report");
      expect(r4[0]?.[0]?.json["method"]).toBe("run.progress");

      const badTokenCtx = makeExecuteContext({
        params: { operation: "getRunProgress", runId: "r_1" },
        httpRequest: async () => ({
          statusCode: 401,
          body: {
            error: {
              code: -32002,
              message: "unauthorized",
              data: { code: "unauthorized", group: "refused" },
            },
          },
        }),
      });
      let tokenErr: unknown;
      try {
        await node.execute.call(badTokenCtx);
      } catch (error) {
        tokenErr = error;
      }
      expect(tokenErr).toBeInstanceOf(NodeOperationError);
      const tokenMeta = readCollieErrorMetadata(tokenErr);
      expect(tokenMeta?.code).toBe("unauthorized");
      expect(tokenMeta?.group).toBe("refused");
      expect((tokenMeta?.fix ?? "").length).toBeGreaterThan(0);

      const badVersionCtx = makeExecuteContext({
        params: { operation: "getRunProgress", runId: "r_1" },
        credentials: {
          baseUrl: "http://127.0.0.1:7801",
          token: "test-token-0000",
          protocolVersion: "99.0",
        },
        httpRequest: async () => ({
          statusCode: 400,
          body: {
            jsonrpc: "2.0",
            id: 1,
            error: {
              code: -32001,
              message: "protocol version 99.0 is not supported",
              data: { code: "protocol_version_unsupported", group: "program" },
            },
          },
        }),
      });
      let versionErr: unknown;
      try {
        await node.execute.call(badVersionCtx);
      } catch (error) {
        versionErr = error;
      }
      expect(versionErr).toBeInstanceOf(NodeOperationError);
      const versionMeta = readCollieErrorMetadata(versionErr);
      expect(versionMeta?.code).toBe("protocol_version_unsupported");
      expect(versionMeta?.group).toBe("program");
      expect((versionMeta?.fix ?? "").length).toBeGreaterThan(0);
    },
  );

  test(
    "trigger fails closed without stored secret, preserves secret on idempotent re-subscribe, verifies HMAC, and deduplicates",
    { tags: ["story:11-249", "integration:n8n/notification"] },
    async () => {
      const trigger = new GoodfetchCollieTrigger();
      const staticData: IDataObject = {};
      const fakeSecret = `whsec_${Buffer.alloc(32, 0x42).toString("base64")}`;
      const rpcCalls: string[] = [];
      let subscribeCount = 0;

      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- partial n8n IHookFunctions test harness.
      const hookCtx = {
        getNodeWebhookUrl: () => "https://hooks.example.com/n8n-trigger",
        getNodeParameter: (name: string, fallbackValue?: unknown) =>
          name === "channel" ? "n8n-ch-1" : name === "events" ? ["run_finished"] : fallbackValue,
        getCredentials: async () => ({
          baseUrl: "http://127.0.0.1:7801",
          token: "test-token-0000",
          protocolVersion: "0.1",
        }),
        getWorkflowStaticData: () => staticData,
        getNode: () => TEST_NODE,
        helpers: {
          httpRequest: async (req: IHttpRequestOptions) => {
            const body = expectRecord(req.body, "hook request body");
            const method = typeof body["method"] === "string" ? body["method"] : "";
            rpcCalls.push(method);
            if (method === "notification.subscribe") {
              subscribeCount += 1;
              return {
                statusCode: 200,
                body: {
                  jsonrpc: "2.0",
                  id: 1,
                  result:
                    subscribeCount === 1
                      ? { channel: "n8n-ch-1", created: true, secret: fakeSecret }
                      : { channel: "n8n-ch-1", created: false },
                },
              };
            }
            return {
              statusCode: 200,
              body: { jsonrpc: "2.0", id: 1, result: { channel: "n8n-ch-1", removed: true } },
            };
          },
        },
      } as unknown as IHookFunctions;

      const now = Math.floor(Date.now() / 1000);
      const eventId = "evt_01JN8NTRIGGER0000000000001";
      const payload: IDataObject = {
        type: "notification.run_finished",
        timestamp: new Date(now * 1000).toISOString(),
        format: "0.1",
        data: {
          event_id: eventId,
          event: "run_finished",
          run_id: "r_01JN8NRUN00000000000000001",
          schedule_id: null,
          product: "collie",
          link: null,
        },
      };
      const rawBody = JSON.stringify(payload);
      const validSig = signStandardWebhookV1(fakeSecret, eventId, now, rawBody);

      const makeWebhookCtx = (
        sig: string,
        dataStore: IDataObject = staticData,
      ): IWebhookFunctions =>
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- partial n8n IWebhookFunctions test harness.
        ({
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
          getWorkflowStaticData: () => dataStore,
          getNode: () => TEST_NODE,
          helpers: {
            httpRequest: async () => ({}),
            returnJsonArray: (items: IDataObject[]) => items.map((json) => ({ json })),
          },
        }) as unknown as IWebhookFunctions;

      // S2: missing stored secret MUST fail closed (reject delivery).
      const emptyStatic: IDataObject = {};
      const rejectedMissingSecret = await trigger.webhook.call(
        makeWebhookCtx(validSig, emptyStatic),
      );
      expect(rejectedMissingSecret.workflowData).toBeUndefined();

      expect(await trigger.webhookMethods.default.checkExists.call(hookCtx)).toBe(false);
      expect(await trigger.webhookMethods.default.create.call(hookCtx)).toBe(true);
      expect(await trigger.webhookMethods.default.checkExists.call(hookCtx)).toBe(true);
      expect(staticData["webhookSecret"]).toBe(fakeSecret);

      // Idempotent re-subscribe (`created: false`, no secret) preserves stored secret.
      expect(await trigger.webhookMethods.default.create.call(hookCtx)).toBe(true);
      expect(staticData["webhookSecret"]).toBe(fakeSecret);

      // Invalid signature is rejected (fires 0 times).
      const badRes = await trigger.webhook.call(makeWebhookCtx("v1,AAAA"));
      expect(badRes.workflowData).toBeUndefined();

      // First valid delivery fires once.
      const firstRes = await trigger.webhook.call(makeWebhookCtx(validSig));
      expect(firstRes.workflowData?.[0]?.length).toBe(1);

      // Duplicate delivery with same event_id does not fire a second time.
      const dupRes = await trigger.webhook.call(makeWebhookCtx(validSig));
      expect(dupRes.workflowData).toBeUndefined();

      expect(await trigger.webhookMethods.default.delete.call(hookCtx)).toBe(true);
      expect(await trigger.webhookMethods.default.checkExists.call(hookCtx)).toBe(false);
      expect(rpcCalls).toEqual([
        "notification.subscribe",
        "notification.subscribe",
        "notification.unsubscribe",
      ]);
    },
  );

  test(
    "n8n-scanner Live check passes locally using real n8n-node lint and flags runtime deps",
    { tags: ["story:11-249"] },
    async () => {
      const localOutcome = await runN8nScanner({ localOnly: true, packageDir: N8N_PACKAGE_ROOT });
      expect(localOutcome.exitCode).toBe(0);

      const prePublish = await runN8nScanner({
        packageDir: N8N_PACKAGE_ROOT,
        fetchImpl: async () => new Response("Not Found", { status: 404 }),
      });
      expect(prePublish.exitCode).toBe(0);

      const badIssues = validatePublishedManifest({
        name: "@goodfetch/n8n-nodes-collie",
        dependencies: { axios: "^1.0.0" },
        keywords: ["n8n-community-node-package"],
        n8n: { strict: true, n8nNodesApiVersion: 1 },
      });
      expect(badIssues.length).toBeGreaterThan(0);
    },
  );
});
