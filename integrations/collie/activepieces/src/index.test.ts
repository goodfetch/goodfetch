// Unit tests for Activepieces piece `goodfetch-collie` (ADRs 0027, 0070, 0072,
// 0090, 0095; spec #608, issue 1409): official `@activepieces/pieces-framework`
// `Piece` validation, per-operation actions via `createMockActionContext`,
// complete Collie catalog Error code mapping with `fix`, refused token errors,
// `start-recipe` `RecipeStartParams` alignment, and fail-closed `notification`
// trigger verification + idempotent re-subscribe + single-fire deduplication.
import { createHmac } from "node:crypto";
import { readFileSync } from "node:fs";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  createMockActionContext,
  Piece,
  type Store,
  type TriggerHookContext,
  type TestOrRunHookContext,
  type TriggerStrategy,
} from "@activepieces/pieces-framework";
import { describe, expect, test } from "vitest";
import { lintActivepiecesPiece } from "../scripts/activepieces-lint.ts";
import {
  ActivepiecesCollieError,
  getRunProgressAction,
  getRunReportAction,
  goodfetchCollieAuth,
  goodfetchColliePiece,
  mapCatalogErrorToActivepieces,
  notificationTrigger,
  startCatalogRunAction,
  startRecipeAction,
} from "./index.ts";

const PACKAGE_ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const REPO_ROOT = resolve(PACKAGE_ROOT, "../../..");

function readJson(path: string): Record<string, unknown> {
  const parsed: unknown = JSON.parse(readFileSync(path, "utf8"));
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new TypeError(`expected JSON object at ${path}`);
  }
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(parsed)) {
    out[k] = v;
  }
  return out;
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

function createMemoryStore(): Store {
  const map = new Map<string, unknown>();
  return {
    async put<T>(key: string, value: T): Promise<T> {
      map.set(key, value);
      return value;
    },
    async get<T>(key: string): Promise<T | null> {
      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- in-memory test Store generic lookup.
      return map.has(key) ? (map.get(key) as T) : null;
    },
    async delete(key: string): Promise<void> {
      map.delete(key);
    },
  };
}

interface RecordedRpcRequest {
  readonly headers: IncomingMessage["headers"];
  readonly body: Record<string, unknown>;
}

async function startStubRpcServer(
  handler: (req: RecordedRpcRequest) => { readonly status: number; readonly body: unknown },
): Promise<{
  readonly url: string;
  readonly requests: RecordedRpcRequest[];
  readonly close: () => Promise<void>;
}> {
  const requests: RecordedRpcRequest[] = [];
  const server = createServer((req: IncomingMessage, res: ServerResponse) => {
    const chunks: Buffer[] = [];
    req.on("data", (chunk: Buffer) => chunks.push(chunk));
    req.on("end", () => {
      const raw = Buffer.concat(chunks).toString("utf8");
      const parsed: unknown = raw.length > 0 ? JSON.parse(raw) : {};
      const bodyObj: Record<string, unknown> = {};
      if (typeof parsed === "object" && parsed !== null && !Array.isArray(parsed)) {
        for (const [k, v] of Object.entries(parsed)) {
          bodyObj[k] = v;
        }
      }
      const record: RecordedRpcRequest = { headers: req.headers, body: bodyObj };
      requests.push(record);
      const reply = handler(record);
      res.writeHead(reply.status, { "Content-Type": "application/json" });
      res.end(JSON.stringify(reply.body));
    });
  });
  await new Promise<void>((resolveListen) => server.listen(0, "127.0.0.1", resolveListen));
  const address = server.address();
  if (address === null || typeof address === "string") {
    throw new Error("failed to bind loopback HTTP server");
  }
  return {
    url: `http://127.0.0.1:${address.port}`,
    requests,
    close: () => new Promise<void>((resolveClose) => server.close(() => resolveClose())),
  };
}

type CollieActionAuth = Parameters<typeof getRunProgressAction.run>[0]["auth"];

function makeAuth(
  baseUrl: string,
  token = "test-token-0000",
  protocolVersion = "0.1",
): CollieActionAuth {
  return {
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- Activepieces AppConnectionType enum string literal.
    type: "CUSTOM_AUTH" as CollieActionAuth["type"],
    props: { baseUrl, token, protocolVersion },
  };
}

function makeActionCtx<T extends { readonly run: (ctx: never) => unknown }>(
  _action: T,
  auth: CollieActionAuth,
  propsValue: Record<string, unknown>,
): Parameters<T["run"]>[0] {
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- Activepieces createMockActionContext widens propsValue to InputPropertyMap.
  return {
    ...createMockActionContext({ propsValue }),
    auth,
    propsValue,
  } as unknown as Parameters<T["run"]>[0];
}

function makeTriggerHookContext(params: {
  readonly baseUrl: string;
  readonly channel: string;
  readonly events?: string[] | undefined;
  readonly webhookUrl: string;
  readonly store: Store;
}): TriggerHookContext<
  typeof goodfetchCollieAuth,
  typeof notificationTrigger.props,
  TriggerStrategy.WEBHOOK
> {
  const base = createMockActionContext({
    propsValue: {
      channel: params.channel,
      events: params.events ?? ["run_finished"],
    },
  });
  return {
    ...base,
    auth: makeAuth(params.baseUrl),
    propsValue: {
      channel: params.channel,
      events: params.events ?? ["run_finished"],
    },
    store: params.store,
    webhookUrl: params.webhookUrl,
    payload: { body: {}, headers: {}, queryParams: {} },
  };
}

function makeTriggerRunContext(params: {
  readonly baseUrl: string;
  readonly channel: string;
  readonly events?: string[] | undefined;
  readonly store: Store;
  readonly body: unknown;
  readonly rawBody: string;
  readonly headers: Record<string, string>;
}): TestOrRunHookContext<
  typeof goodfetchCollieAuth,
  typeof notificationTrigger.props,
  TriggerStrategy.WEBHOOK
> {
  const base = createMockActionContext({
    propsValue: {
      channel: params.channel,
      events: params.events ?? ["run_finished"],
    },
  });
  return {
    ...base,
    auth: makeAuth(params.baseUrl),
    propsValue: {
      channel: params.channel,
      events: params.events ?? ["run_finished"],
    },
    store: params.store,
    webhookUrl: "https://hooks.example.com/ap-trigger",
    payload: {
      body: params.body,
      rawBody: params.rawBody,
      headers: params.headers,
      queryParams: {},
    },
  };
}

describe("goodfetch-collie (Activepieces piece)", () => {
  test(
    "is an official @activepieces/pieces-framework Piece passing lint with all four actions and trigger",
    { tags: ["story:11-252"] },
    () => {
      const lint = lintActivepiecesPiece(PACKAGE_ROOT);
      expect(lint.violations).toEqual([]);
      expect(lint.ok).toBe(true);
      expect(goodfetchColliePiece).toBeInstanceOf(Piece);
      expect(Object.keys(goodfetchColliePiece.actions())).toEqual([
        "start-recipe",
        "start-catalog-run",
        "get-run-report",
        "get-run-progress",
      ]);
      expect(Object.keys(goodfetchColliePiece.triggers())).toEqual(["notification"]);
    },
  );

  test(
    "maps every Error code in Collie's catalog to an Activepieces error with code, group, and fix",
    { tags: ["story:11-252"] },
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
        const mapped = mapCatalogErrorToActivepieces({
          code: name,
          group,
          description: typeof description === "string" ? description : undefined,
        });
        expect(mapped).toBeInstanceOf(ActivepiecesCollieError);
        expect(mapped.code).toBe(name);
        expect(mapped.group).toBe(group);
        expect(mapped.fix.length).toBeGreaterThan(0);
      }
    },
  );

  test(
    "runs all four actions via @activepieces/pieces-framework createMockActionContext and never sends unknown recipe.start fields",
    {
      tags: [
        "story:11-252",
        "integration:activepieces/start-recipe",
        "integration:activepieces/start-catalog-run",
        "integration:activepieces/get-run-report",
        "integration:activepieces/get-run-progress",
      ],
    },
    async () => {
      const server = await startStubRpcServer((req) => {
        if (req.headers["authorization"] === "Bearer wrong-token") {
          return {
            status: 401,
            body: {
              jsonrpc: "2.0",
              id: 1,
              error: {
                code: -32002,
                message: "unauthorized",
                data: { code: "unauthorized", group: "refused" },
              },
            },
          };
        }
        return {
          status: 200,
          body: { jsonrpc: "2.0", id: 1, result: { ok: true, method: req.body["method"] } },
        };
      });

      try {
        const auth = makeAuth(server.url);
        const r1 = await startRecipeAction.run(
          makeActionCtx(startRecipeAction, auth, {
            recipe: "checkout",
            tags: ["env:test"],
            settings: { timeout: { navigation: 15000 } },
          }),
        );
        const r2 = await startCatalogRunAction.run(
          makeActionCtx(startCatalogRunAction, auth, {
            target: { kind: "extract", url: "https://example.com", schema: {} },
          }),
        );
        const r3 = await getRunReportAction.run(
          makeActionCtx(getRunReportAction, auth, { runId: "r_1" }),
        );
        const r4 = await getRunProgressAction.run(
          makeActionCtx(getRunProgressAction, auth, { runId: "r_1" }),
        );

        expect(server.requests.map((r) => r.body["method"])).toEqual([
          "recipe.start",
          "catalog_run.start",
          "run.report",
          "run.progress",
        ]);
        // P10: `recipe.start` must match `RecipeStartParams` (no unknown `params` field).
        const recipeReqParams = expectRecord(
          server.requests[0]?.body["params"],
          "recipe.start params",
        );
        expect(recipeReqParams["params"]).toBeUndefined();
        expect(recipeReqParams["recipe"]).toBe("checkout");
        expect(recipeReqParams["detached"]).toBe(true);
        expect(recipeReqParams["tags"]).toEqual(["env:test"]);
        expect(recipeReqParams["settings"]).toEqual({ timeout: { navigation: 15000 } });

        expect(server.requests[0]?.headers["goodfetch-product"]).toBe("collie");
        expect(server.requests[0]?.headers["goodfetch-form"]).toBe("catalog");
        expect(expectRecord(r1, "r1")["method"]).toBe("recipe.start");
        expect(expectRecord(r2, "r2")["method"]).toBe("catalog_run.start");
        expect(expectRecord(r3, "r3")["method"]).toBe("run.report");
        expect(expectRecord(r4, "r4")["method"]).toBe("run.progress");

        let caught: unknown;
        try {
          await getRunProgressAction.run(
            makeActionCtx(getRunProgressAction, makeAuth(server.url, "wrong-token"), {
              runId: "r_1",
            }),
          );
        } catch (error) {
          caught = error;
        }
        if (!(caught instanceof ActivepiecesCollieError)) {
          throw new Error(`expected ActivepiecesCollieError, got ${String(caught)}`);
        }
        expect(caught.code).toBe("unauthorized");
        expect(caught.group).toBe("refused");
        expect(caught.fix.length).toBeGreaterThan(0);
      } finally {
        await server.close();
      }
    },
  );

  test(
    "notification trigger fails closed when secret is missing, preserves secret on idempotent re-subscribe, verifies HMAC, and deduplicates",
    { tags: ["story:11-252", "integration:activepieces/notification"] },
    async () => {
      const fakeSecret = `whsec_${Buffer.alloc(32, 0x43).toString("base64")}`;
      let subscribeCount = 0;
      const server = await startStubRpcServer((req) => {
        if (req.body["method"] === "notification.subscribe") {
          subscribeCount += 1;
          return {
            status: 200,
            body: {
              jsonrpc: "2.0",
              id: 1,
              result:
                subscribeCount === 1
                  ? { channel: "ap-ch-1", created: true, secret: fakeSecret }
                  : { channel: "ap-ch-1", created: false },
            },
          };
        }
        return {
          status: 200,
          body: { jsonrpc: "2.0", id: 1, result: { channel: "ap-ch-1", removed: true } },
        };
      });

      try {
        const now = Math.floor(Date.now() / 1000);
        const eventId = "evt_01JAPTRIGGER00000000000001";
        const body = {
          type: "notification.run_finished",
          timestamp: new Date(now * 1000).toISOString(),
          format: "0.1",
          data: {
            event_id: eventId,
            event: "run_finished",
            run_id: "r_01JAPRUN000000000000000001",
            schedule_id: null,
            product: "collie",
            link: null,
          },
        };
        const rawBody = JSON.stringify(body);
        const sig = signStandardWebhookV1(fakeSecret, eventId, now, rawBody);

        // S2: missing stored secret MUST fail closed (reject delivery).
        const emptyStore = createMemoryStore();
        const rejectedWithoutSecret = await notificationTrigger.run(
          makeTriggerRunContext({
            baseUrl: server.url,
            channel: "ap-ch-1",
            store: emptyStore,
            body,
            rawBody,
            headers: {
              "webhook-id": eventId,
              "webhook-timestamp": String(now),
              "webhook-signature": sig,
            },
          }),
        );
        expect(rejectedWithoutSecret).toEqual([]);

        const store = createMemoryStore();
        await notificationTrigger.onEnable(
          makeTriggerHookContext({
            baseUrl: server.url,
            channel: "ap-ch-1",
            webhookUrl: "https://hooks.example.com/ap-trigger",
            store,
          }),
        );

        // Idempotent re-subscribe (created: false, no secret returned) preserves existing stored secret.
        await notificationTrigger.onEnable(
          makeTriggerHookContext({
            baseUrl: server.url,
            channel: "ap-ch-1",
            webhookUrl: "https://hooks.example.com/ap-trigger",
            store,
          }),
        );

        // Bad signature is rejected (0 items).
        const badItems = await notificationTrigger.run(
          makeTriggerRunContext({
            baseUrl: server.url,
            channel: "ap-ch-1",
            store,
            body,
            rawBody,
            headers: {
              "webhook-id": eventId,
              "webhook-timestamp": String(now),
              "webhook-signature": "v1,AAAA",
            },
          }),
        );
        expect(badItems).toEqual([]);

        // First valid delivery fires once.
        const firstItems = await notificationTrigger.run(
          makeTriggerRunContext({
            baseUrl: server.url,
            channel: "ap-ch-1",
            store,
            body,
            rawBody,
            headers: {
              "webhook-id": eventId,
              "webhook-timestamp": String(now),
              "webhook-signature": sig,
            },
          }),
        );
        expect(firstItems.length).toBe(1);

        // Duplicate delivery with same event_id does not fire again.
        const dupItems = await notificationTrigger.run(
          makeTriggerRunContext({
            baseUrl: server.url,
            channel: "ap-ch-1",
            store,
            body,
            rawBody,
            headers: {
              "webhook-id": eventId,
              "webhook-timestamp": String(now),
              "webhook-signature": sig,
            },
          }),
        );
        expect(dupItems).toEqual([]);

        await notificationTrigger.onDisable(
          makeTriggerHookContext({
            baseUrl: server.url,
            channel: "ap-ch-1",
            webhookUrl: "https://hooks.example.com/ap-trigger",
            store,
          }),
        );
        expect(server.requests.map((r) => r.body["method"])).toEqual([
          "notification.subscribe",
          "notification.subscribe",
          "notification.unsubscribe",
        ]);
      } finally {
        await server.close();
      }
    },
  );
});
