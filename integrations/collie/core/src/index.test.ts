// Unit tests for `@goodfetch/collie-integrations-core` (ADRs 0027, 0056, 0070,
// 0072, 0095; spec #608, issue 1403): generated-surface kept-off check, zero
// Foxhound dependencies, complete catalog Error code mapping table, and
// Standard Webhooks delivery verification.
import { createHmac } from "node:crypto";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";
import {
  INTEGRATION_ACTIONS,
  INTEGRATION_FORM,
  INTEGRATION_PRODUCT,
  INTEGRATION_TRIGGERS,
  IntegrationHttpClient,
  IntegrationToolError,
  KEPT_OFF_INTEGRATION_OPERATIONS,
  mapCatalogError,
  verifyNotificationDelivery,
} from "./index.ts";

const REPO_ROOT = resolve(fileURLToPath(new URL("../../../..", import.meta.url)));

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

describe("@goodfetch/collie-integrations-core", () => {
  test(
    "announces form catalog and Product collie, exposes no kept-off entry, and depends on no Foxhound package",
    { tags: ["story:11-254", "story:11-257"] },
    () => {
      expect(INTEGRATION_PRODUCT).toBe("collie");
      expect(INTEGRATION_FORM).toBe("catalog");
      expect(INTEGRATION_ACTIONS.map((action) => action.id)).toEqual([
        "start-recipe",
        "start-catalog-run",
        "get-run-report",
        "get-run-progress",
      ]);
      expect(INTEGRATION_TRIGGERS.map((trigger) => trigger.id)).toEqual(["notification"]);

      const keptOff = new Set(KEPT_OFF_INTEGRATION_OPERATIONS);
      for (const action of INTEGRATION_ACTIONS) {
        expect(keptOff.has(action.operation), `${action.operation} must not be kept-off`).toBe(
          false,
        );
      }
      expect(keptOff.has("notification.subscribe")).toBe(true);
      expect(keptOff.has("notification.unsubscribe")).toBe(true);

      const pkg = readJson(join(REPO_ROOT, "integrations", "collie", "core", "package.json"));
      const deps = pkg["dependencies"];
      const depNames =
        typeof deps === "object" && deps !== null && !Array.isArray(deps) ? Object.keys(deps) : [];
      for (const dep of depNames) {
        expect(dep.toLowerCase(), `Collie Integration must not depend on ${dep}`).not.toContain(
          "foxhound",
        );
      }
    },
  );

  test(
    "maps every Error code in Collie's catalog to code, group, and fix",
    { tags: ["story:11-254"] },
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
        const mapped = mapCatalogError({
          code: name,
          group,
          description: typeof description === "string" ? description : undefined,
        });
        expect(mapped.code).toBe(name);
        expect(mapped.group).toBe(group);
        expect(mapped.fix.length).toBeGreaterThan(0);
      }
    },
  );

  test(
    "verifies thin Standard Webhooks deliveries with the SDK helper and rejects bad signatures",
    { tags: ["story:11-255"] },
    async () => {
      const fakeSecret = `whsec_${Buffer.alloc(32, 0x41).toString("base64")}`;
      const eventId = "evt_01JTESTINTEGRATION00000001";
      const signedAt = 1_760_000_000;
      const body = JSON.stringify({
        type: "notification.run_finished",
        timestamp: new Date(signedAt * 1000).toISOString(),
        format: "0.1",
        data: {
          event_id: eventId,
          event: "run_finished",
          run_id: "r_01JTESTRUN0000000000000001",
          schedule_id: null,
          product: "collie",
          link: null,
        },
      });
      const sig = signStandardWebhookV1(fakeSecret, eventId, signedAt, body);
      const headers = {
        "webhook-id": eventId,
        "webhook-timestamp": String(signedAt),
        "webhook-signature": sig,
      };

      const verified = await verifyNotificationDelivery(body, headers, fakeSecret, {
        at: new Date(signedAt * 1000),
      });
      expect(verified.type).toBe("notification.run_finished");

      await expect(
        verifyNotificationDelivery(
          body,
          { ...headers, "webhook-signature": "v1,AAAA" },
          fakeSecret,
          {
            at: new Date(signedAt * 1000),
          },
        ),
      ).rejects.toThrow("signature");
    },
  );

  test(
    "sends form catalog and Product collie headers and surfaces refused token and protocol errors with fix",
    { tags: ["story:11-254"] },
    async () => {
      let seenHeaders: Headers | undefined;
      const fakeFetch: typeof fetch = async (_url, init) => {
        seenHeaders = new Headers(init?.headers);
        return new Response(
          JSON.stringify({
            jsonrpc: "2.0",
            id: 1,
            error: {
              code: -32001,
              message: "protocol version 99.0 is not supported",
              data: {
                code: "protocol_version_unsupported",
                group: "program",
              },
            },
          }),
          { status: 400, headers: { "Content-Type": "application/json" } },
        );
      };

      const client = new IntegrationHttpClient({
        baseUrl: "http://127.0.0.1:7801",
        token: "test-token-0000",
        protocolVersion: "99.0",
        fetchImpl: fakeFetch,
      });

      let caught: unknown;
      try {
        await client.getRunProgress("r_test");
      } catch (error) {
        caught = error;
      }

      expect(seenHeaders?.get("Goodfetch-Product")).toBe("collie");
      expect(seenHeaders?.get("Goodfetch-Form")).toBe("catalog");
      expect(seenHeaders?.get("Goodfetch-Protocol")).toBe("99.0");
      expect(seenHeaders?.get("Authorization")).toBe("Bearer test-token-0000");

      if (!(caught instanceof IntegrationToolError)) {
        throw new Error(`expected IntegrationToolError, got ${String(caught)}`);
      }
      expect(caught.code).toBe("protocol_version_unsupported");
      expect(caught.group).toBe("program");
      expect(caught.fix.length).toBeGreaterThan(0);
    },
  );
});
