// Pre-merge integration tests for Activepieces piece `goodfetch-collie`
// (ADRs 0027, 0070, 0072, 0090, 0095; spec #608, issue 1409).
//
// 1. Builds `goodfetch-collie` (`pnpm run build`) and dynamically imports the
//    compiled `dist/index.js` `@activepieces/pieces-framework` `Piece`.
// 2. Runs each compiled action via `createMockActionContext` against the real
//    Collie Daemon HTTP listener (`goodfetch-test-env --daemon=collie
//    --http-listener`), including `start-recipe` with `tags` and `settings`
//    (verifying `RecipeStartParams` alignment — `run_target_unbuilt`, never
//    `invalid_params`), refused token, and refused protocol version.
// 3. Exercises **real Daemon webhook delivery** through the compiled
//    `notification` trigger: subscribes to a local loopback HTTP receiver,
//    starts and finishes a Run on the Collie Daemon so `SubscriptionRunEndObserver`
//    delivers a real HMAC-SHA256 signed webhook, and passes that real HTTP
//    delivery into the compiled trigger twice (verifying signature + single-fire).
// 4. Exercises `goodfetch-scripted-engine --mode http` for Error codes the
//    Proving ground cannot cause (`model_unavailable`).
// 5. When a container engine answers, executes the compiled `goodfetch-collie`
//    piece inside the pinned Activepieces container (`--network=none`).
import { execFileSync, spawn } from "node:child_process";
import { chmodSync, mkdirSync, writeFileSync } from "node:fs";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  createMockActionContext,
  Piece,
  type Store,
  type TestOrRunHookContext,
  type TriggerHookContext,
  type TriggerStrategy,
} from "@activepieces/pieces-framework";
import { startActivepieces, test, toolPath } from "@goodfetch/test-support";
import { beforeAll, describe, expect } from "vitest";
import type {
  getRunProgressAction,
  goodfetchCollieAuth,
  notificationTrigger,
} from "../src/index.ts";

const PACKAGE_ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const DIST_ENTRY = join(PACKAGE_ROOT, "dist", "index.js");

type CollieActionAuth = Parameters<typeof getRunProgressAction.run>[0]["auth"];

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

function normalizeStringHeaders(
  headers: Record<string, string | string[] | undefined>,
): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(headers)) {
    if (typeof v === "string") {
      out[k] = v;
    } else if (Array.isArray(v) && v.length > 0 && v[0] !== undefined) {
      out[k] = v[0];
    }
  }
  return out;
}

function makeAuth(baseUrl: string, token: string, protocolVersion = "0.1"): CollieActionAuth {
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

async function expectActivepiecesError(
  promise: Promise<unknown>,
): Promise<{ readonly code: string; readonly group: string; readonly fix: string }> {
  let caught: unknown;
  try {
    await promise;
  } catch (error) {
    caught = error;
  }
  const code =
    typeof caught === "object" && caught !== null ? Reflect.get(caught, "code") : undefined;
  const group =
    typeof caught === "object" && caught !== null ? Reflect.get(caught, "group") : undefined;
  const fix =
    typeof caught === "object" && caught !== null ? Reflect.get(caught, "fix") : undefined;
  if (typeof code !== "string" || typeof group !== "string" || typeof fix !== "string") {
    throw new Error(`expected ActivepiecesCollieError, got ${String(caught)}`);
  }
  expect(code.length).toBeGreaterThan(0);
  expect(fix.length).toBeGreaterThan(0);
  return { code, group, fix };
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
      "loads compiled dist/ piece, executes actions + real Daemon webhook delivery + scripted-engine errors, and exercises Activepieces container when available",
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
      async ({ daemon, testEnv }) => {
        const daemonHttp = daemon.http;
        const httpToken = daemon.http_token;
        if (daemonHttp === undefined || httpToken === undefined) {
          throw new Error("expected Daemon HTTP listener URL and token from goodfetch-test-env");
        }

        // 1. Dynamically load the COMPILED `dist/index.js` Activepieces piece (P12 + S7).
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- runtime dynamic import of compiled dist/index.js piece.
        const compiled = (await import(
          pathToFileURL(DIST_ENTRY).href
        )) as typeof import("../src/index.ts");
        expect(compiled.goodfetchColliePiece).toBeInstanceOf(Piece);
        const compiledActions = compiled.goodfetchColliePiece.actions();
        const compiledTriggers = compiled.goodfetchColliePiece.triggers();
        expect(Object.keys(compiledActions)).toEqual([
          "start-recipe",
          "start-catalog-run",
          "get-run-report",
          "get-run-progress",
        ]);
        expect(Object.keys(compiledTriggers)).toEqual(["notification"]);

        const auth = makeAuth(daemonHttp, httpToken);
        const rawClient = compiled.createCollieClient(auth);
        const started = await rawClient.call("run.start", {});
        const runId = started["run_id"];
        if (typeof runId !== "string" || runId.length === 0) {
          throw new Error(`expected run_id from run.start, got ${JSON.stringify(started)}`);
        }

        // 2. Execute each compiled action via `@activepieces/pieces-framework` `createMockActionContext`.
        const progress = expectRecord(
          await compiled.getRunProgressAction.run(
            makeActionCtx(compiled.getRunProgressAction, auth, { runId }),
          ),
          "get-run-progress output",
        );
        expect(progress["run_id"]).toBe(runId);

        await rawClient.call("run.finish", { run_id: runId });
        const report = expectRecord(
          await compiled.getRunReportAction.run(
            makeActionCtx(compiled.getRunReportAction, auth, { runId }),
          ),
          "get-run-report output",
        );
        expect(report["run_id"]).toBe(runId);

        // P10: `start-recipe` with tags & settings parses cleanly into `RecipeStartParams` (`run_target_unbuilt`, never `invalid_params`).
        const recipeErr = await expectActivepiecesError(
          compiled.startRecipeAction.run(
            makeActionCtx(compiled.startRecipeAction, auth, {
              recipe: "nonexistent-recipe",
              tags: ["surface:activepieces"],
              settings: { timeout: { navigation: 15_000 } },
            }),
          ),
        );
        expect(recipeErr.code).toBe("run_target_unbuilt");
        expect(recipeErr.group).toBe("unavailable");

        const catalogErr = await expectActivepiecesError(
          compiled.startCatalogRunAction.run(
            makeActionCtx(compiled.startCatalogRunAction, auth, {
              target: { kind: "nonexistent_target" },
            }),
          ),
        );
        expect(["program", "unavailable", "refused"]).toContain(catalogErr.group);

        // 3. Refused token and refused hello version arrive as Activepieces errors with fix.
        const tokenErr = await expectActivepiecesError(
          compiled.getRunProgressAction.run(
            makeActionCtx(compiled.getRunProgressAction, makeAuth(daemonHttp, "wrong-token-0000"), {
              runId,
            }),
          ),
        );
        expect(tokenErr.group).toBe("refused");

        const versionErr = await expectActivepiecesError(
          compiled.getRunProgressAction.run(
            makeActionCtx(compiled.getRunProgressAction, makeAuth(daemonHttp, httpToken, "99.0"), {
              runId,
            }),
          ),
        );
        expect(versionErr.code).toBe("protocol_version_unsupported");

        // 4. P13: Real Daemon webhook delivery -> compiled `notification` trigger.
        const receiver = await startWebhookReceiver();
        const store = createMemoryStore();
        try {
          const hookCtx: TriggerHookContext<
            typeof goodfetchCollieAuth,
            typeof notificationTrigger.props,
            TriggerStrategy.WEBHOOK
          > = {
            ...createMockActionContext({
              propsValue: { channel: "ap-real-daemon-trigger", events: ["run_finished"] },
            }),
            auth,
            propsValue: { channel: "ap-real-daemon-trigger", events: ["run_finished"] },
            store,
            webhookUrl: receiver.url,
            payload: { body: {}, headers: {}, queryParams: {} },
          };
          await compiled.notificationTrigger.onEnable(hookCtx);
          const storedSecret = await store.get<string>("goodfetch_collie_secret");
          expect(typeof storedSecret).toBe("string");
          expect(storedSecret?.startsWith("whsec_")).toBe(true);

          // Start and finish a real Run so the Daemon's `SubscriptionRunEndObserver`
          // delivers a real Standard Webhooks HMAC-signed POST to `receiver.url`.
          const notifiedRun = await rawClient.call("run.start", {});
          const notifiedRunId = String(notifiedRun["run_id"]);
          await rawClient.call("run.finish", { run_id: notifiedRunId });

          const delivered = await receiver.nextDelivery();
          expect(delivered.body["type"]).toBe("notification.run_finished");
          expect(
            typeof delivered.headers["webhook-signature"] === "string" &&
              delivered.headers["webhook-signature"].startsWith("v1,"),
          ).toBe(true);

          const runCtx: TestOrRunHookContext<
            typeof goodfetchCollieAuth,
            typeof notificationTrigger.props,
            TriggerStrategy.WEBHOOK
          > = {
            ...createMockActionContext({
              propsValue: { channel: "ap-real-daemon-trigger", events: ["run_finished"] },
            }),
            auth,
            propsValue: { channel: "ap-real-daemon-trigger", events: ["run_finished"] },
            store,
            webhookUrl: receiver.url,
            payload: {
              body: delivered.body,
              rawBody: delivered.rawBody,
              headers: normalizeStringHeaders(delivered.headers),
              queryParams: {},
            },
          };

          const firstFire = await compiled.notificationTrigger.run(runCtx);
          expect(firstFire.length).toBe(1);
          const secondFire = await compiled.notificationTrigger.run(runCtx);
          expect(secondFire).toEqual([]);

          await compiled.notificationTrigger.onDisable(hookCtx);
        } finally {
          await receiver.close();
        }

        // 5. P13: Scripted engine (`--mode http`) delivers an unprocurable Error code
        //    (`model_unavailable`) through the compiled Activepieces action.
        const scriptedBin = toolPath(testEnv.tools, "scripted_engine");
        const scriptPath = join(testEnv.folder, "ap-scripted-http.json");
        writeFileSync(
          scriptPath,
          JSON.stringify({
            description: "Returns model_unavailable to Activepieces compiled action",
            steps: [
              {
                expect: {
                  method: "recipe.start",
                  params: { recipe: "ap-model-err", detached: true },
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
          token: "ap-scripted-token",
          env: testEnv.env,
        });
        try {
          const scriptedErr = await expectActivepiecesError(
            compiled.startRecipeAction.run(
              makeActionCtx(
                compiled.startRecipeAction,
                makeAuth(scripted.baseUrl, scripted.token),
                { recipe: "ap-model-err" },
              ),
            ),
          );
          expect(scriptedErr.code).toBe("model_unavailable");
          expect(scriptedErr.group).toBe("model");
        } finally {
          scripted.kill();
        }

        // 6. P12: When Docker/Podman is available, bundle the compiled `dist/index.js`
        //    piece and execute its actions inside the pinned Activepieces container
        //    (`--network=none`) against the real Collie Daemon.
        await runPinnedActivepiecesContainerCheck(testEnv.folder, daemonHttp, httpToken, runId);
      },
    );
  },
);

async function runPinnedActivepiecesContainerCheck(
  workFolder: string,
  daemonHttp: string,
  httpToken: string,
  runId: string,
): Promise<void> {
  if (!hasContainerEngine()) {
    return;
  }
  const bundleDir = join(workFolder, "goodfetch-piece-bundle");
  const viteConfigPath = join(workFolder, "bundle-piece.vite.config.mjs");
  mkdirSync(bundleDir, { recursive: true, mode: 0o755 });
  chmodSync(bundleDir, 0o755);
  writeFileSync(
    viteConfigPath,
    `export default {
  logLevel: "error",
  build: {
    outDir: ${JSON.stringify(bundleDir)},
    ssr: ${JSON.stringify(DIST_ENTRY)},
    minify: false,
    rollupOptions: {
      external: [/^node:/],
      output: { format: "es", entryFileNames: "goodfetch-collie.mjs" },
    },
  },
  ssr: { noExternal: true },
};
`,
    "utf8",
  );
  execFileSync("pnpm", ["exec", "vite", "build", "--config", viteConfigPath], {
    cwd: PACKAGE_ROOT,
    stdio: "pipe",
    timeout: 60_000,
  });
  chmodSync(join(bundleDir, "goodfetch-collie.mjs"), 0o644);

  const ap = await startActivepieces({
    daemonHttp,
    readOnlyMounts: [{ hostPath: bundleDir, containerPath: "/tmp/goodfetch-piece" }],
  });
  try {
    const pieceRunnerCode = [
      "import { goodfetchColliePiece, getRunProgressAction, getRunReportAction, startRecipeAction } from '/tmp/goodfetch-piece/goodfetch-collie.mjs';",
      "export const code = async (inputs) => {",
      "  const auth = { type: 'CUSTOM_AUTH', props: { baseUrl: inputs.url, token: inputs.token, protocolVersion: '0.1' } };",
      "  const mk = (propsValue) => ({ auth, propsValue });",
      "  const progress = await getRunProgressAction.run(mk({ runId: inputs.runId }));",
      "  const report = await getRunReportAction.run(mk({ runId: inputs.runId }));",
      "  let recipeErrorCode = null;",
      "  try { await startRecipeAction.run(mk({ recipe: 'nonexistent-recipe', tags: ['surface:activepieces'], settings: { timeout: { navigation: 15000 } } })); } catch (e) { recipeErrorCode = e.code ?? null; }",
      "  return { actions: Object.keys(goodfetchColliePiece.actions()), progressRunId: progress.run_id, reportRunId: report.run_id, recipeErrorCode };",
      "};",
    ].join("\n");
    const result = await ap.runCodeStep({
      code: pieceRunnerCode,
      input: { url: daemonHttp, token: httpToken, runId },
    });
    expect(result.status).toBe("SUCCEEDED");
    expect(result.output).toMatchObject({
      progressRunId: runId,
      reportRunId: runId,
      recipeErrorCode: "run_target_unbuilt",
    });
  } finally {
    await ap.stop();
  }
}
