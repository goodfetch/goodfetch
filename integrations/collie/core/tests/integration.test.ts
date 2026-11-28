// Integration tests for `@goodfetch/collie-integrations-core` (ADRs 0027,
// 0056, 0070, 0095; spec #608, issue 1403): drives the shared generated
// Integration layer over both the real Collie Daemon HTTP JSON-RPC listener
// (`goodfetch-test-env --daemon=collie --http-listener`) and
// `goodfetch-scripted-engine --mode http` for Error codes the Proving ground
// cannot cause.
import { spawn } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { test, toolPath } from "@goodfetch/test-support";
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

async function startScriptedHttpEngine(options: {
  readonly executable: string;
  readonly scriptPath: string;
  readonly token: string;
  readonly env: Readonly<Record<string, string>>;
}): Promise<{
  readonly baseUrl: string;
  readonly token: string;
  readonly waitExit: () => Promise<number | null>;
  readonly kill: () => void;
}> {
  const child = spawn(
    options.executable,
    ["--script", options.scriptPath, "--mode", "http", "--token", options.token],
    {
      env: { ...options.env },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  let stderr = "";
  child.stderr.on("data", (chunk: string) => {
    stderr += chunk;
  });

  const announced = await new Promise<{ readonly address: string; readonly token: string }>(
    (resolveReady, rejectReady) => {
      let buf = "";
      const onData = (chunk: string): void => {
        buf += chunk;
        const nl = buf.indexOf("\n");
        if (nl !== -1) {
          child.stdout.off("data", onData);
          try {
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
          } catch (err) {
            rejectReady(err);
          }
        }
      };
      child.stdout.on("data", onData);
      child.once("exit", (code) => {
        rejectReady(
          new Error(`goodfetch-scripted-engine exited (${String(code)}) before ready: ${stderr}`),
        );
      });
    },
  );

  return {
    baseUrl: `http://${announced.address}`,
    token: announced.token,
    waitExit: () =>
      new Promise<number | null>((resolveExit) => {
        if (child.exitCode !== null) {
          resolveExit(child.exitCode);
          return;
        }
        child.once("exit", (code) => resolveExit(code));
      }),
    kill: () => {
      child.kill("SIGKILL");
    },
  };
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
    async ({ daemon, testEnv }) => {
      if (daemon.http === undefined || daemon.http_token === undefined) {
        throw new Error("expected Daemon HTTP listener URL and token from goodfetch-test-env");
      }

      const client = new IntegrationHttpClient({
        baseUrl: daemon.http,
        token: daemon.http_token,
      });

      // 1. Start a Run with a run-layer setting over HTTP -> origin is `form` (P9)
      //    and read its progress and report.
      const started = await client.call("run.start", {
        settings: { timeout: { navigation: 45_000 } },
      });
      const runId = started["run_id"];
      if (typeof runId !== "string" || runId.length === 0) {
        throw new Error(`expected run_id string from run.start, got ${JSON.stringify(started)}`);
      }
      const explained = await client.call("config.explain", {
        run_id: runId,
        keys: ["timeout.navigation"],
      });
      const explainedSettings = explained["settings"];
      expect(Array.isArray(explainedSettings)).toBe(true);
      const navEntry: unknown = Array.isArray(explainedSettings)
        ? explainedSettings.find(
            (e) =>
              typeof e === "object" && e !== null && Reflect.get(e, "key") === "timeout.navigation",
          )
        : undefined;
      expect(
        typeof navEntry === "object" && navEntry !== null
          ? Reflect.get(navEntry, "layer")
          : undefined,
      ).toBe("run");
      expect(
        typeof navEntry === "object" && navEntry !== null
          ? Reflect.get(navEntry, "origin")
          : undefined,
      ).toBe("form");

      const progress = await client.getRunProgress(runId);
      expect(progress["run_id"]).toBe(runId);
      await client.call("run.finish", { run_id: runId });
      const report = await client.getRunReport(runId);
      expect(report["run_id"]).toBe(runId);

      // 2. P10: Detached recipe.start with tags & settings parses cleanly into
      //    `RecipeStartParams` (returns `run_target_unbuilt`, NEVER `invalid_params`).
      const recipeErr = await expectToolError(
        client.startRecipe({
          recipe: "nonexistent-recipe",
          tags: ["source:integration-core"],
          settings: { timeout: { navigation: 15_000 } },
        }),
      );
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
      //    loopback cdp_url -> cdp_url_not_allowed,
      //    model-only locked key (`proxy.kind`, `authority=not_by_model`) -> setting_locked,
      //    Secret binding (`secrets.placeholders`) -> setting_locked.
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

      // P13: model-locked key (`proxy.kind` has Authority::NotByModel) returns `setting_locked`.
      const modelLockErr = await expectToolError(
        client.call("run.start", {
          settings: {
            proxy: { kind: "mobile" },
          },
        }),
      );
      expect(modelLockErr.code).toBe("setting_locked");
      expect(modelLockErr.group).toBe("refused");

      // Secret binding (`secrets.placeholders`) also returns `setting_locked`.
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

      // 6. P13: Scripted engine as a Daemon with an HTTP listener (`--mode http`)
      //    returns Error codes the Proving ground cannot cause (`model_unavailable`,
      //    `budget_exceeded`), surfaced with code, group, and fix.
      const scriptedBin = toolPath(testEnv.tools, "scripted_engine");
      const modelScriptPath = join(testEnv.folder, "integration-core-scripted-model.json");
      writeFileSync(
        modelScriptPath,
        JSON.stringify({
          description: "Returns model_unavailable over HTTP listener",
          steps: [
            {
              expect: {
                method: "recipe.start",
                params: { recipe: "scripted-model-err", detached: true },
              },
            },
            {
              reply: {
                error: {
                  code: -32004,
                  message: "upstream model provider returned 503",
                  data: {
                    code: "model_unavailable",
                    group: "model",
                    fix: "Switch to another configured model or retry later.",
                  },
                },
              },
            },
          ],
        }),
        "utf8",
      );

      const scriptedModel = await startScriptedHttpEngine({
        executable: scriptedBin,
        scriptPath: modelScriptPath,
        token: "scripted-http-token-1",
        env: testEnv.env,
      });
      try {
        const scriptedClient = new IntegrationHttpClient({
          baseUrl: scriptedModel.baseUrl,
          token: scriptedModel.token,
        });
        const modelErr = await expectToolError(
          scriptedClient.startRecipe({ recipe: "scripted-model-err" }),
        );
        expect(modelErr.code).toBe("model_unavailable");
        expect(modelErr.group).toBe("model");
        expect(modelErr.fix).toContain("model");
      } finally {
        scriptedModel.kill();
        await scriptedModel.waitExit();
      }

      const budgetScriptPath = join(testEnv.folder, "integration-core-scripted-budget.json");
      writeFileSync(
        budgetScriptPath,
        JSON.stringify({
          description: "Returns budget_exceeded over HTTP listener",
          steps: [
            {
              expect: {
                method: "run.progress",
                params: { run_id: "r_budget" },
              },
            },
            {
              reply: {
                error: {
                  code: -32002,
                  message: "run currency limit reached",
                  data: {
                    code: "budget_exceeded",
                    group: "refused",
                    fix: "Raise the Run budget in Operator policy or start a smaller Run.",
                  },
                },
              },
            },
          ],
        }),
        "utf8",
      );

      const scriptedBudget = await startScriptedHttpEngine({
        executable: scriptedBin,
        scriptPath: budgetScriptPath,
        token: "scripted-http-token-2",
        env: testEnv.env,
      });
      try {
        const scriptedClient = new IntegrationHttpClient({
          baseUrl: scriptedBudget.baseUrl,
          token: scriptedBudget.token,
        });
        const budgetErr = await expectToolError(scriptedClient.getRunProgress("r_budget"));
        expect(budgetErr.code).toBe("budget_exceeded");
        expect(budgetErr.group).toBe("refused");
        expect(budgetErr.fix).toContain("budget");
      } finally {
        scriptedBudget.kill();
        await scriptedBudget.waitExit();
      }
    },
  );
});
