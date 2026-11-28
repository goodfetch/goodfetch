// Unit tests for Activepieces piece `goodfetch-collie` (ADRs 0027, 0070, 0072,
// 0090, 0095; spec #608, issue 1409): piece lint, per-operation actions via
// `createMockActionContext`, complete Collie catalog Error code mapping with
// `fix`, refused token and protocol version errors, and `notification` trigger
// signature verification + single-fire deduplication.
import { createHmac } from "node:crypto";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";
import { lintActivepiecesPiece } from "../scripts/activepieces-lint.ts";
import {
  ActivepiecesCollieError,
  createMemoryStore,
  createMockActionContext,
  getRunProgressAction,
  getRunReportAction,
  goodfetchColliePiece,
  mapCatalogErrorToActivepieces,
  notificationTrigger,
  startCatalogRunAction,
  startRecipeAction,
} from "./index.ts";

const PACKAGE_ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const REPO_ROOT = resolve(PACKAGE_ROOT, "../../..");

const TEST_AUTH = {
  baseUrl: "http://127.0.0.1:7801",
  token: "test-token-0000",
  protocolVersion: "0.1",
} as const;

const BAD_TOKEN_FETCH: typeof fetch = async () =>
  new Response(
    JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      error: {
        code: -32002,
        message: "unauthorized",
        data: { code: "unauthorized", group: "refused" },
      },
    }),
    { status: 401, headers: { "Content-Type": "application/json" } },
  );

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

describe("goodfetch-collie (Activepieces piece)", () => {
  test(
    "passes Activepieces piece lint and declares all four actions plus notification trigger",
    { tags: ["story:11-252"] },
    () => {
      const lint = lintActivepiecesPiece(PACKAGE_ROOT);
      expect(lint.violations).toEqual([]);
      expect(lint.ok).toBe(true);
      expect(goodfetchColliePiece.name).toBe("goodfetch-collie");
      expect(goodfetchColliePiece.actions.map((a) => a.name)).toEqual([
        "start-recipe",
        "start-catalog-run",
        "get-run-report",
        "get-run-progress",
      ]);
      expect(goodfetchColliePiece.triggers.map((t) => t.name)).toEqual(["notification"]);
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
    "runs all four per-operation actions via createMockActionContext and surfaces refused token with fix",
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
      const seenMethods: string[] = [];
      const okFetch: typeof fetch = async (_url, init) => {
        const headers = new Headers(init?.headers);
        expect(headers.get("Goodfetch-Product")).toBe("collie");
        expect(headers.get("Goodfetch-Form")).toBe("catalog");
        expect(headers.get("Authorization")).toBe("Bearer test-token-0000");
        const bodyText = typeof init?.body === "string" ? init.body : "{}";
        const payload: unknown = JSON.parse(bodyText);
        const method =
          typeof payload === "object" && payload !== null && "method" in payload
            ? String(payload.method)
            : "";
        seenMethods.push(method);
        return new Response(
          JSON.stringify({ jsonrpc: "2.0", id: 1, result: { ok: true, method } }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      };

      const r1 = await startRecipeAction.run(
        createMockActionContext({
          auth: TEST_AUTH,
          propsValue: { recipe: "checkout" },
          fetchImpl: okFetch,
        }),
      );
      const r2 = await startCatalogRunAction.run(
        createMockActionContext({
          auth: TEST_AUTH,
          propsValue: { target: { kind: "url", url: "https://example.com" } },
          fetchImpl: okFetch,
        }),
      );
      const r3 = await getRunReportAction.run(
        createMockActionContext({
          auth: TEST_AUTH,
          propsValue: { runId: "r_1" },
          fetchImpl: okFetch,
        }),
      );
      const r4 = await getRunProgressAction.run(
        createMockActionContext({
          auth: TEST_AUTH,
          propsValue: { runId: "r_1" },
          fetchImpl: okFetch,
        }),
      );

      expect(seenMethods).toEqual([
        "recipe.start",
        "catalog_run.start",
        "run.report",
        "run.progress",
      ]);
      expect(r1["method"]).toBe("recipe.start");
      expect(r2["method"]).toBe("catalog_run.start");
      expect(r3["method"]).toBe("run.report");
      expect(r4["method"]).toBe("run.progress");

      let caught: unknown;
      try {
        await getRunProgressAction.run(
          createMockActionContext({
            auth: TEST_AUTH,
            propsValue: { runId: "r_1" },
            fetchImpl: BAD_TOKEN_FETCH,
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
    },
  );

  test(
    "notification trigger subscribes, verifies signature, fires once per event_id, and unsubscribes",
    { tags: ["story:11-252", "integration:activepieces/notification"] },
    async () => {
      const store = createMemoryStore();
      const fakeSecret = `whsec_${Buffer.alloc(32, 0x43).toString("base64")}`;
      const rpcMethods: string[] = [];

      const hookFetch: typeof fetch = async (_url, init) => {
        const bodyText = typeof init?.body === "string" ? init.body : "{}";
        const parsed: unknown = JSON.parse(bodyText);
        const method =
          typeof parsed === "object" && parsed !== null && "method" in parsed
            ? String(parsed.method)
            : "";
        rpcMethods.push(method);
        if (method === "notification.subscribe") {
          return new Response(
            JSON.stringify({
              jsonrpc: "2.0",
              id: 1,
              result: { channel: "ap-ch-1", created: true, secret: fakeSecret },
            }),
            { status: 200, headers: { "Content-Type": "application/json" } },
          );
        }
        return new Response(
          JSON.stringify({
            jsonrpc: "2.0",
            id: 1,
            result: { channel: "ap-ch-1", removed: true },
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      };

      await notificationTrigger.onEnable({
        auth: TEST_AUTH,
        propsValue: { channel: "ap-ch-1", events: ["run_finished"] },
        webhookUrl: "https://hooks.example.com/ap-trigger",
        store,
        fetchImpl: hookFetch,
      });

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

      // Bad signature is rejected (0 items).
      const badItems = await notificationTrigger.run({
        auth: TEST_AUTH,
        propsValue: { channel: "ap-ch-1", events: ["run_finished"] },
        store,
        payload: {
          body,
          rawBody,
          headers: {
            "webhook-id": eventId,
            "webhook-timestamp": String(now),
            "webhook-signature": "v1,AAAA",
          },
        },
      });
      expect(badItems).toEqual([]);

      // First valid delivery fires once.
      const firstItems = await notificationTrigger.run({
        auth: TEST_AUTH,
        propsValue: { channel: "ap-ch-1", events: ["run_finished"] },
        store,
        payload: {
          body,
          rawBody,
          headers: {
            "webhook-id": eventId,
            "webhook-timestamp": String(now),
            "webhook-signature": sig,
          },
        },
      });
      expect(firstItems.length).toBe(1);

      // Duplicate delivery with same event_id does not fire again.
      const dupItems = await notificationTrigger.run({
        auth: TEST_AUTH,
        propsValue: { channel: "ap-ch-1", events: ["run_finished"] },
        store,
        payload: {
          body,
          rawBody,
          headers: {
            "webhook-id": eventId,
            "webhook-timestamp": String(now),
            "webhook-signature": sig,
          },
        },
      });
      expect(dupItems).toEqual([]);

      await notificationTrigger.onDisable({
        auth: TEST_AUTH,
        propsValue: { channel: "ap-ch-1", events: ["run_finished"] },
        webhookUrl: "https://hooks.example.com/ap-trigger",
        store,
        fetchImpl: hookFetch,
      });
      expect(rpcMethods).toEqual(["notification.subscribe", "notification.unsubscribe"]);
    },
  );
});
