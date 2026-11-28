// Unit tests for `@goodfetch/n8n-nodes-collie` (ADRs 0027, 0070, 0072, 0090,
// 0095; spec #608, issue 1405): strict `n8n-node lint` verified-node rules,
// complete Collie catalog Error code mapping, refused token and protocol
// version errors, trigger webhook signature verification + single-fire dedup,
// and the `n8n-scanner` Live check.
import { createHmac } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";
import { runN8nScanner, validatePublishedManifest } from "../../../../ci/live/n8n-scanner.ts";
import { lintN8nPackage } from "../scripts/n8n-node-lint.ts";
import {
  GoodfetchCollie,
  GoodfetchCollieApi,
  GoodfetchCollieTrigger,
  mapCatalogErrorToN8n,
  N8nCollieError,
  type IDataObject,
  type IExecuteFunctions,
  type IHookFunctions,
  type IHttpRequestOptions,
  type INodeDescriptor,
  type IWebhookFunctions,
} from "./index.ts";

const PACKAGE_ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const REPO_ROOT = resolve(PACKAGE_ROOT, "../../..");

const TEST_NODE: INodeDescriptor = {
  id: "collie-1",
  name: "Goodfetch Collie",
  type: "CUSTOM.goodfetchCollie",
  typeVersion: 1,
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

function makeExecuteContext(options: {
  readonly params: Record<string, unknown>;
  readonly credentials?: IDataObject;
  readonly httpRequest: (req: IHttpRequestOptions) => Promise<unknown>;
  readonly continueOnFail?: boolean;
}): IExecuteFunctions {
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
  };
}

describe("@goodfetch/n8n-nodes-collie", () => {
  test(
    "passes strict n8n-node lint verified-node rules (zero runtime deps, no process.env/fs/builtins)",
    { tags: ["story:11-249"] },
    () => {
      const result = lintN8nPackage(PACKAGE_ROOT);
      expect(result.violations).toEqual([]);
      expect(result.ok).toBe(true);

      const creds = new GoodfetchCollieApi();
      expect(creds.name).toBe("goodfetchCollieApi");
      expect(creds.authenticate?.properties.headers?.["Goodfetch-Product"]).toBe("collie");
      expect(creds.authenticate?.properties.headers?.["Goodfetch-Form"]).toBe("catalog");

      const node = new GoodfetchCollie();
      expect(node.description.name).toBe("goodfetchCollie");
      expect(node.description.usableAsTool).toBe(true);

      const trigger = new GoodfetchCollieTrigger();
      expect(trigger.description.name).toBe("goodfetchCollieTrigger");
    },
  );

  test(
    "n8n-node lint rejects runtime dependencies and process.env or fs access",
    { tags: ["story:11-249"] },
    () => {
      const dir = mkdtempSync(join(tmpdir(), "goodfetch-n8n-lint-bad-"));
      try {
        mkdirSync(join(dir, "src"), { recursive: true });
        writeFileSync(
          join(dir, "package.json"),
          JSON.stringify({
            name: "@goodfetch/n8n-nodes-bad",
            keywords: ["n8n-community-node-package"],
            dependencies: { axios: "^1.0.0" },
            n8n: { n8nNodesApiVersion: 1, strict: true, nodes: ["a"], credentials: ["b"] },
          }),
          "utf8",
        );
        writeFileSync(
          join(dir, "src", "bad.ts"),
          'import fs from "node:fs";\nexport const x = process.env.SECRET ?? fs;\n',
          "utf8",
        );
        const result = lintN8nPackage(dir);
        expect(result.ok).toBe(false);
        const rules = result.violations.map((v) => v.rule);
        expect(rules).toContain("n8n-nodes-base/verified-node-no-runtime-dependencies");
        expect(rules).toContain("n8n-nodes-base/no-process-env");
        expect(rules).toContain("n8n-nodes-base/no-node-builtins");
      } finally {
        rmSync(dir, { recursive: true, force: true });
      }
    },
  );

  test(
    "maps every Error code in Collie's catalog to an n8n error with code, group, and fix",
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
        expect(mapped).toBeInstanceOf(N8nCollieError);
        expect(mapped.name).toBe("NodeOperationError");
        expect(mapped.code).toBe(name);
        expect(mapped.group).toBe(group);
        expect(mapped.fix.length).toBeGreaterThan(0);
      }
    },
  );

  test(
    "executes all four actions and surfaces refused token and hello version errors with code, group, and fix",
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
      const seenMethods: string[] = [];

      const okCtx = (params: Record<string, unknown>) =>
        makeExecuteContext({
          params,
          httpRequest: async (req) => {
            expect(req.headers?.["Goodfetch-Product"]).toBe("collie");
            expect(req.headers?.["Goodfetch-Form"]).toBe("catalog");
            expect(req.headers?.["Authorization"]).toBe("Bearer test-token-0000");
            const method = String(req.body?.["method"]);
            seenMethods.push(method);
            return {
              statusCode: 200,
              body: { jsonrpc: "2.0", id: 1, result: { ok: true, method } },
            };
          },
        });

      const r1 = await node.execute.call(okCtx({ operation: "startRecipe", recipe: "checkout" }));
      const r2 = await node.execute.call(
        okCtx({
          operation: "startCatalogRun",
          catalogTarget: '{"kind":"url","url":"https://example.com"}',
        }),
      );
      const r3 = await node.execute.call(okCtx({ operation: "getRunReport", runId: "r_1" }));
      const r4 = await node.execute.call(okCtx({ operation: "getRunProgress", runId: "r_1" }));

      expect(seenMethods).toEqual([
        "recipe.start",
        "catalog_run.start",
        "run.report",
        "run.progress",
      ]);
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
      if (!(tokenErr instanceof N8nCollieError)) {
        throw new Error(`expected N8nCollieError, got ${String(tokenErr)}`);
      }
      expect(tokenErr.code).toBe("unauthorized");
      expect(tokenErr.group).toBe("refused");
      expect(tokenErr.fix.length).toBeGreaterThan(0);

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
      if (!(versionErr instanceof N8nCollieError)) {
        throw new Error(`expected N8nCollieError, got ${String(versionErr)}`);
      }
      expect(versionErr.code).toBe("protocol_version_unsupported");
      expect(versionErr.group).toBe("program");
      expect(versionErr.fix.length).toBeGreaterThan(0);
    },
  );

  test(
    "trigger subscribes, verifies Standard Webhooks signature via Web Crypto, fires once per event_id, and unsubscribes",
    { tags: ["story:11-249", "integration:n8n/notification"] },
    async () => {
      const trigger = new GoodfetchCollieTrigger();
      const staticData: IDataObject = {};
      const fakeSecret = `whsec_${Buffer.alloc(32, 0x42).toString("base64")}`;
      const rpcCalls: string[] = [];

      const hookCtx: IHookFunctions = {
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
          httpRequest: async (req) => {
            const method = String(req.body?.["method"]);
            rpcCalls.push(method);
            if (method === "notification.subscribe") {
              return {
                statusCode: 200,
                body: {
                  jsonrpc: "2.0",
                  id: 1,
                  result: { channel: "n8n-ch-1", created: true, secret: fakeSecret },
                },
              };
            }
            return {
              statusCode: 200,
              body: { jsonrpc: "2.0", id: 1, result: { channel: "n8n-ch-1", removed: true } },
            };
          },
        },
      };

      expect(await trigger.webhookMethods.default.checkExists.call(hookCtx)).toBe(false);
      expect(await trigger.webhookMethods.default.create.call(hookCtx)).toBe(true);
      expect(await trigger.webhookMethods.default.checkExists.call(hookCtx)).toBe(true);
      expect(staticData["webhookSecret"]).toBe(fakeSecret);

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

      const makeWebhookCtx = (sig: string): IWebhookFunctions => ({
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
        getNode: () => TEST_NODE,
        helpers: {
          httpRequest: async () => ({}),
          returnJsonArray: (items) => items.map((json) => ({ json })),
        },
      });

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
      expect(rpcCalls).toEqual(["notification.subscribe", "notification.unsubscribe"]);
    },
  );

  test(
    "n8n-scanner Live check passes locally, handles pre-publish 404, and flags runtime deps on published manifest",
    { tags: ["story:11-249"] },
    async () => {
      const localOutcome = await runN8nScanner({ localOnly: true, packageDir: PACKAGE_ROOT });
      expect(localOutcome.exitCode).toBe(0);

      const prePublish = await runN8nScanner({
        packageDir: PACKAGE_ROOT,
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
