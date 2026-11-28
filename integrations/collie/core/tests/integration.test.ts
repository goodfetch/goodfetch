// Integration tests for `@goodfetch/collie-integrations-core` (ADRs 0027,
// 0056, 0070, 0095; spec #608, issue 1403): drives the shared generated
// Integration layer over the real Collie Daemon's HTTP JSON-RPC listener
// (`goodfetch-test-env --daemon=collie --http-listener`).
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { IntegrationHttpClient, IntegrationToolError } from "../src/index.ts";

async function expectToolError(promise: Promise<unknown>): Promise<IntegrationToolError> {
  let caught: unknown;
  try {
    await promise;
  } catch (error) {
    caught = error;
  }
  expect(caught).toBeInstanceOf(IntegrationToolError);
  if (!(caught instanceof IntegrationToolError)) {
    throw new Error(`expected IntegrationToolError, got ${String(caught)}`);
  }
  expect(caught.code.length).toBeGreaterThan(0);
  expect(caught.fix.length).toBeGreaterThan(0);
  return caught;
}

describe("@goodfetch/collie-integrations-core against a real Collie Daemon HTTP listener", () => {
  test.override("withDaemon", true);
  test.override("withHttpListener", true);
  test.override("withBrowser", true);

  test(
    "starts detached runs, reads report and progress, subscribes/unsubscribes idempotently, and enforces acting-for-model rules",
    {
      tags: [
        "story:11-254",
        "story:11-255",
        "story:11-257",
        "operation:notification.subscribe",
        "operation:notification.unsubscribe",
      ],
    },
    async ({ daemon }) => {
      if (daemon.http === undefined || daemon.http_token === undefined) {
        throw new Error("expected Daemon HTTP listener URL and token from goodfetch-test-env");
      }

      const client = new IntegrationHttpClient({
        baseUrl: daemon.http,
        token: daemon.http_token,
      });

      // 1. Start a Run and read its report and progress over HTTP.
      const started = await client.call("run.start", {});
      const runId = started["run_id"];
      if (typeof runId !== "string" || runId.length === 0) {
        throw new Error(`expected run_id string from run.start, got ${JSON.stringify(started)}`);
      }
      const progress = await client.getRunProgress(runId);
      expect(progress["run_id"]).toBe(runId);
      await client.call("run.finish", { run_id: runId });
      const report = await client.getRunReport(runId);
      expect(report["run_id"]).toBe(runId);

      // 2. Detached recipe.start and catalog_run.start reach the Daemon over HTTP with form catalog.
      const recipeErr = await expectToolError(client.startRecipe({ recipe: "nonexistent-recipe" }));
      expect(recipeErr.code).toBe("run_target_unbuilt");
      expect(recipeErr.group).toBe("unavailable");

      const catalogRunErr = await expectToolError(
        client.startCatalogRun({
          target: { kind: "nonexistent_target" },
        }),
      );
      expect(["program", "unavailable", "refused"]).toContain(catalogRunErr.group);

      // 3. Idempotent notification.subscribe and notification.unsubscribe.
      const sub1 = await client.subscribeNotification({
        channel: "integration-trigger-1",
        url: "https://hooks.example.com/collie-trigger",
        events: ["run_finished", "run_failed"],
      });
      expect(sub1.channel).toBe("integration-trigger-1");
      expect(sub1.created).toBe(true);
      expect(sub1.secret?.startsWith("whsec_")).toBe(true);

      const sub2 = await client.subscribeNotification({
        channel: "integration-trigger-1",
        url: "https://hooks.example.com/collie-trigger",
        events: ["run_finished", "run_failed"],
      });
      expect(sub2.channel).toBe("integration-trigger-1");
      expect(sub2.created).toBe(false);
      expect(sub2.secret).toBeUndefined();

      const unsub1 = await client.unsubscribeNotification("integration-trigger-1");
      expect(unsub1).toEqual({ channel: "integration-trigger-1", removed: true });
      const unsub2 = await client.unsubscribeNotification("integration-trigger-1");
      expect(unsub2).toEqual({ channel: "integration-trigger-1", removed: false });

      // 4. Refused token and refused hello protocol version arrive with code, group, and fix.
      const badTokenClient = new IntegrationHttpClient({
        baseUrl: daemon.http,
        token: "wrong-token-0000",
      });
      const tokenErr = await expectToolError(badTokenClient.getRunProgress(runId));
      expect(tokenErr.group).toBe("refused");

      const badVersionClient = new IntegrationHttpClient({
        baseUrl: daemon.http,
        token: daemon.http_token,
        protocolVersion: "99.0",
      });
      const versionErr = await expectToolError(badVersionClient.getRunProgress(runId));
      expect(versionErr.code).toBe("protocol_version_unsupported");

      // 5. Client acting for a model (ADR 0056 / ADR 0092):
      //    loopback cdp_url -> cdp_url_not_allowed, Secret binding -> setting_locked.
      const actingRun = await client.call("run.start", {});
      const actingRunId = actingRun["run_id"];
      if (typeof actingRunId !== "string" || actingRunId.length === 0) {
        throw new Error(`expected run_id string, got ${JSON.stringify(actingRun)}`);
      }
      const cdpErr = await expectToolError(
        client.call("run.new_session", {
          run_id: actingRunId,
          source: { kind: "cdp_url", url: "ws://127.0.0.1:9222/devtools/browser/test" },
        }),
      );
      expect(cdpErr.code).toBe("cdp_url_not_allowed");
      expect(cdpErr.group).toBe("refused");

      const exclusiveBorrowErr = await expectToolError(
        client.call("run.new_session", {
          run_id: actingRunId,
          source: {
            kind: "cdp_url",
            url: "ws://127.0.0.1:9222/devtools/browser/test",
            exclusive: true,
          },
        }),
      );
      expect(exclusiveBorrowErr.group).toBe("refused");
      await client.call("run.finish", { run_id: actingRunId });

      const secretLockErr = await expectToolError(
        client.call("run.start", {
          settings: {
            "secrets.placeholders": {
              api_key: { reference: "env:TEST_API_KEY", domains: ["example.com"] },
            },
          },
        }),
      );
      expect(secretLockErr.code).toBe("setting_locked");
      expect(secretLockErr.group).toBe("refused");
    },
  );
});
