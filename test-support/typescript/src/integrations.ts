// Pinned n8n and Activepieces Integration harnesses (ADR 0070; issue 426).
// Both images are pinned by multi-platform digest in
// `ci/integrations/images.toml` and pulled before ADR 0043's outbound-traffic
// deny by `ci/integrations/fetch.sh` (`just fetch`). Each runs inside
// `--network=none` for physical offline isolation, bridging loopback HTTP
// across bind-mounted Unix sockets using the Node.js runtime already shipped
// in both images, so it works identically on Docker, Podman, and the Linux
// Release suite's `goodfetch-egress` network namespace.
import { execFile, execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import {
  connect as netConnect,
  createServer,
  type Server as NetServer,
  type Socket,
} from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { parse as parseToml } from "smol-toml";
import { TestSupportError } from "./errors.ts";

/** One pinned OCI container image from `ci/integrations/images.toml`. */
export interface PinnedImage {
  readonly repository: string;
  readonly tag: string;
  readonly digest: string;
  /** `<repository>@<digest>`, the immutable reference passed to `docker`/`podman`. */
  readonly reference: string;
}

/** The pinned n8n and Activepieces images declared in `ci/integrations/images.toml`. */
export interface IntegrationImages {
  readonly n8n: PinnedImage;
  readonly activepieces: PinnedImage;
}

/** A container engine that answers on this host. */
export type ContainerEngine = "docker" | "podman";

/** Result of running an n8n workflow inside the pinned n8n image. */
export interface N8nWorkflowResult {
  readonly exitCode: number;
  readonly stdout: string;
  readonly stderr: string;
}

/** Options for `runN8nWorkflow`. */
export interface N8nWorkflowOptions {
  /** Workflow JSON object to import and execute. */
  readonly workflow: Readonly<Record<string, unknown>>;
  /** Workflow ID passed to `n8n execute --id`. */
  readonly workflowId: string;
  /**
   * Optional Daemon HTTP listener base URL (`http://127.0.0.1:<port>`) to
   * bridge into the `--network=none` container on the same loopback port.
   */
  readonly daemonHttp?: string | undefined;
  /** Optional credentials JSON array to import before running the workflow. */
  readonly credentials?: ReadonlyArray<Readonly<Record<string, unknown>>> | undefined;
  /**
   * Optional host directory of built custom n8n nodes/credentials to mount into
   * the container and load via `N8N_CUSTOM_EXTENSIONS`.
   */
  readonly customExtensionsDir?: string | undefined;
  /** Extra environment variables passed to the n8n container. */
  readonly env?: Readonly<Record<string, string>> | undefined;
  /** Container image reference; defaults to `ci/integrations/images.toml`'s `[n8n]`. */
  readonly image?: string | undefined;
  /** Container engine; auto-detected when absent. */
  readonly engine?: ContainerEngine | undefined;
  /** Maximum runtime in milliseconds; defaults to 120_000. */
  readonly timeoutMs?: number | undefined;
}

/** Input for running a single Activepieces CODE step in the started container. */
export interface ActivepiecesCodeStep {
  readonly code: string;
  readonly input: Readonly<Record<string, unknown>>;
  readonly timeoutMs?: number | undefined;
}

/** Result of running an Activepieces CODE step via `/api/v1/sample-data/test-step`. */
export interface ActivepiecesStepResult {
  readonly status: string;
  readonly output: unknown;
  readonly failedStep: unknown;
}

/** A running single-container Activepieces instance (SQLite + in-memory queue). */
export interface StartedActivepieces {
  readonly url: string;
  readonly port: number;
  readonly containerId: string;
  /** Creates a flow with one `CODE` action, executes it offline, and returns the step result. */
  runCodeStep(step: ActivepiecesCodeStep): Promise<ActivepiecesStepResult>;
  /** Stops and removes the container and closes the loopback bridge. */
  stop(): Promise<void>;
}

/** Options for `startActivepieces`. */
export interface ActivepiecesStartOptions {
  /**
   * Optional Daemon HTTP listener base URL (`http://127.0.0.1:<port>`) to
   * bridge into the `--network=none` container on the same loopback port.
   */
  readonly daemonHttp?: string | undefined;
  /** Container image reference; defaults to `ci/integrations/images.toml`'s `[activepieces]`. */
  readonly image?: string | undefined;
  /** Container engine; auto-detected when absent. */
  readonly engine?: ContainerEngine | undefined;
  /** Maximum startup wait in milliseconds; defaults to 60_000. */
  readonly readyTimeoutMs?: number | undefined;
}

const SHA256_RE = /^sha256:[0-9a-f]{64}$/;
const REPO_ROOT = resolve(fileURLToPath(new URL("../../..", import.meta.url)));
const DEFAULT_IMAGES_FILE = join(REPO_ROOT, "ci", "integrations", "images.toml");
const ACTIVEPIECES_CONTAINER_PORT = 3000;
const PER_REQUEST_TIMEOUT_CAP_MS = 10_000;

/**
 * Tiny CommonJS TCP <-> Unix-socket bridge executed by the Node.js runtime
 * already shipped inside both pinned images under `--network=none`.
 * - `DAEMON_PORT` + `DAEMON_SOCK`: listens on container `127.0.0.1:<DAEMON_PORT>`
 *   and pipes connections to the host-bound Unix socket `DAEMON_SOCK`.
 * - `AP_SOCK` + `AP_PORT`: listens on Unix socket `AP_SOCK` (mode 0666) and
 *   pipes connections to container `127.0.0.1:<AP_PORT>`.
 */
const CONTAINER_BRIDGE_SCRIPT = [
  'const fs = require("node:fs");',
  'const net = require("node:net");',
  "const pipe = (a, b) => {",
  "  a.pipe(b); b.pipe(a);",
  "  a.on('error', () => b.destroy());",
  "  b.on('error', () => a.destroy());",
  "};",
  "const pending = [];",
  "if (process.env.DAEMON_PORT && process.env.DAEMON_SOCK) {",
  "  const port = Number(process.env.DAEMON_PORT);",
  "  const sock = process.env.DAEMON_SOCK;",
  "  pending.push(new Promise((res, rej) => {",
  "    const srv = net.createServer((c) => pipe(c, net.createConnection(sock)));",
  "    srv.once('error', rej);",
  "    srv.listen(port, '127.0.0.1', res);",
  "  }));",
  "}",
  "if (process.env.AP_SOCK && process.env.AP_PORT) {",
  "  const sock = process.env.AP_SOCK;",
  "  const port = Number(process.env.AP_PORT);",
  "  if (fs.existsSync(sock)) fs.unlinkSync(sock);",
  "  pending.push(new Promise((res, rej) => {",
  "    const srv = net.createServer((c) => pipe(c, net.createConnection({ host: '127.0.0.1', port })));",
  "    srv.once('error', rej);",
  "    srv.listen(sock, () => { fs.chmodSync(sock, 0o666); res(); });",
  "  }));",
  "}",
  "Promise.all(pending).then(() => {",
  "  if (process.env.BRIDGE_READY) fs.writeFileSync(process.env.BRIDGE_READY, 'ok');",
  "}).catch((err) => { console.error(err); process.exit(1); });",
].join("\n");

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readStringField(record: Record<string, unknown>, key: string, context: string): string {
  const value = record[key];
  if (typeof value !== "string" || value.length === 0) {
    throw new TestSupportError(
      "integration_harness_failed",
      `${context}: expected non-empty string field \`${key}\``,
    );
  }
  return value;
}

async function readJsonObject(
  response: Response,
  context: string,
): Promise<Record<string, unknown>> {
  const body: unknown = await response.json();
  if (!isRecord(body)) {
    throw new TestSupportError(
      "integration_harness_failed",
      `${context}: expected a JSON object response`,
    );
  }
  return body;
}

function parsePinnedImage(raw: unknown, name: string, filePath: string): PinnedImage {
  if (!isRecord(raw)) {
    throw new TestSupportError(
      "integration_images_invalid",
      `${filePath} is missing the [${name}] image table`,
    );
  }
  const repository = raw["repository"];
  const tag = raw["tag"];
  const digest = raw["digest"];
  if (typeof repository !== "string" || repository.length === 0) {
    throw new TestSupportError(
      "integration_images_invalid",
      `${filePath} [${name}] is missing a non-empty \`repository\``,
    );
  }
  if (typeof tag !== "string" || tag.length === 0) {
    throw new TestSupportError(
      "integration_images_invalid",
      `${filePath} [${name}] is missing a non-empty \`tag\``,
    );
  }
  if (typeof digest !== "string" || !SHA256_RE.test(digest)) {
    throw new TestSupportError(
      "integration_images_invalid",
      `${filePath} [${name}] must pin \`digest\` as \`sha256:<64 hex>\``,
    );
  }
  return { repository, tag, digest, reference: `${repository}@${digest}` };
}

/** Reads and validates the pinned n8n and Activepieces images from `filePath`. */
export function readIntegrationImages(filePath: string = DEFAULT_IMAGES_FILE): IntegrationImages {
  let text: string;
  try {
    text = readFileSync(filePath, "utf8");
  } catch (error) {
    throw new TestSupportError(
      "integration_images_invalid",
      `cannot read Integration image pins at ${filePath}`,
      { cause: error },
    );
  }
  let parsed: Record<string, unknown>;
  try {
    parsed = parseToml(text);
  } catch (error) {
    throw new TestSupportError("integration_images_invalid", `${filePath} is not valid TOML`, {
      cause: error,
    });
  }
  return {
    n8n: parsePinnedImage(parsed["n8n"], "n8n", filePath),
    activepieces: parsePinnedImage(parsed["activepieces"], "activepieces", filePath),
  };
}

function detectContainerEngine(): ContainerEngine | undefined {
  for (const candidate of ["docker", "podman"] as const) {
    try {
      execFileSync(candidate, ["info"], { stdio: "ignore", timeout: 5_000 });
      return candidate;
    } catch {
      // Try next engine.
    }
  }
  return undefined;
}

function requireEngine(engine?: ContainerEngine): ContainerEngine {
  const resolved = engine ?? detectContainerEngine();
  if (resolved === undefined) {
    throw new TestSupportError(
      "container_engine_missing",
      "no container engine (`docker` or `podman`) answers on this host",
    );
  }
  return resolved;
}

function requireLocalImage(engine: ContainerEngine, image: string): void {
  try {
    execFileSync(engine, ["image", "inspect", image], { stdio: "ignore", timeout: 10_000 });
  } catch (error) {
    throw new TestSupportError(
      "container_image_missing",
      `pinned image ${image} is not present locally; run \`ci/integrations/fetch.sh\` before the outbound-traffic deny`,
      { cause: error },
    );
  }
}

function parseLoopbackPort(baseUrl: string): number {
  let parsed: URL;
  try {
    parsed = new URL(baseUrl);
  } catch (error) {
    throw new TestSupportError("integration_harness_failed", `invalid daemonHttp URL ${baseUrl}`, {
      cause: error,
    });
  }
  const port = Number(parsed.port);
  if (
    parsed.protocol !== "http:" ||
    parsed.hostname !== "127.0.0.1" ||
    !Number.isInteger(port) ||
    port <= 0 ||
    port > 65535
  ) {
    throw new TestSupportError(
      "integration_harness_failed",
      `daemonHttp must be http://127.0.0.1:<port>, got ${baseUrl}`,
    );
  }
  return port;
}

function pipeSockets(a: Socket, b: Socket): void {
  a.pipe(b);
  b.pipe(a);
  a.on("error", () => {
    b.destroy();
  });
  b.on("error", () => {
    a.destroy();
  });
}

async function startUnixToTcpBridge(socketPath: string, targetPort: number): Promise<NetServer> {
  return await new Promise<NetServer>((resolveServer, rejectServer) => {
    const server = createServer((client) => {
      pipeSockets(client, netConnect({ host: "127.0.0.1", port: targetPort }));
    });
    server.once("error", rejectServer);
    server.listen(socketPath, () => {
      chmodSync(socketPath, 0o666);
      resolveServer(server);
    });
  });
}

async function startTcpToUnixBridge(
  socketPath: string,
): Promise<{ readonly server: NetServer; readonly port: number }> {
  return await new Promise((resolveServer, rejectServer) => {
    const server = createServer((client) => {
      pipeSockets(client, netConnect(socketPath));
    });
    server.once("error", rejectServer);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (address === null || typeof address === "string") {
        server.close(() => {
          rejectServer(new Error("could not bind a loopback port for Activepieces bridge"));
        });
        return;
      }
      resolveServer({ server, port: address.port });
    });
  });
}

async function closeServer(server: NetServer | undefined): Promise<void> {
  if (server === undefined) {
    return;
  }
  await new Promise<void>((resolveClose) => {
    server.close(() => {
      resolveClose();
    });
  });
}

function runCommand(
  command: string,
  args: readonly string[],
  timeoutMs: number,
): Promise<{ readonly exitCode: number; readonly stdout: string; readonly stderr: string }> {
  return new Promise((resolveRun, rejectRun) => {
    execFile(
      command,
      [...args],
      { timeout: timeoutMs, maxBuffer: 16 * 1024 * 1024 },
      (error, stdout, stderr) => {
        if (error === null) {
          resolveRun({ exitCode: 0, stdout, stderr });
          return;
        }
        if (error.killed) {
          rejectRun(
            new TestSupportError(
              "integration_harness_failed",
              `${command} timed out after ${timeoutMs}ms: ${stderr.trim()}`,
              { cause: error },
            ),
          );
          return;
        }
        if (typeof error.code !== "number") {
          rejectRun(
            new TestSupportError(
              "integration_harness_failed",
              `${command} failed to spawn: ${error.message}`,
              { cause: error },
            ),
          );
          return;
        }
        resolveRun({ exitCode: error.code, stdout, stderr });
      },
    );
  });
}

async function forceRemoveContainer(engine: ContainerEngine, nameOrId: string): Promise<void> {
  try {
    const removed = await runCommand(engine, ["rm", "-f", nameOrId], 15_000);
    if (removed.exitCode !== 0) {
      process.stderr.write(
        `goodfetch-test-support: ${engine} rm -f ${nameOrId} exited ${removed.exitCode}: ${removed.stderr.trim()}\n`,
      );
    }
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    process.stderr.write(
      `goodfetch-test-support: failed to remove container ${nameOrId}: ${detail}\n`,
    );
  }
}

/** Imports and executes one n8n workflow offline inside the pinned n8n image (`--network=none`). */
export async function runN8nWorkflow(options: N8nWorkflowOptions): Promise<N8nWorkflowResult> {
  const engine = requireEngine(options.engine);
  const image = options.image ?? readIntegrationImages().n8n.reference;
  requireLocalImage(engine, image);
  const containerName = `goodfetch-n8n-${randomUUID()}`;
  const folder = mkdtempSync(join(tmpdir(), "goodfetch-n8n-"));
  chmodSync(folder, 0o777);
  const workflowPath = join(folder, "workflow.json");
  const bridgeScriptPath = join(folder, "bridge.cjs");
  let daemonBridge: NetServer | undefined;
  try {
    writeFileSync(workflowPath, JSON.stringify(options.workflow), "utf8");
    chmodSync(workflowPath, 0o644);
    const args: string[] = [
      "run",
      "--rm",
      "--name",
      containerName,
      "--network=none",
      "-e",
      "N8N_DIAGNOSTICS_ENABLED=false",
      "-e",
      "N8N_VERSION_NOTIFICATIONS_ENABLED=false",
      "-e",
      "N8N_TEMPLATES_ENABLED=false",
      "-e",
      "N8N_HIRING_BANNER_ENABLED=false",
      "-e",
      "N8N_PERSONALIZATION_ENABLED=false",
      "-e",
      "N8N_ENFORCE_SETTINGS_FILE_PERMISSIONS=false",
    ];
    let startupPrefix = "";
    if (options.daemonHttp !== undefined) {
      const daemonPort = parseLoopbackPort(options.daemonHttp);
      daemonBridge = await startUnixToTcpBridge(join(folder, "daemon.sock"), daemonPort);
      writeFileSync(bridgeScriptPath, CONTAINER_BRIDGE_SCRIPT, "utf8");
      chmodSync(bridgeScriptPath, 0o644);
      args.push(
        "-e",
        `DAEMON_PORT=${daemonPort}`,
        "-e",
        "DAEMON_SOCK=/tmp/bridge/daemon.sock",
        "-e",
        "BRIDGE_READY=/tmp/bridge/ready",
      );
      startupPrefix =
        "node /tmp/bridge/bridge.cjs & until [ -f /tmp/bridge/ready ]; do sleep 0.05; done; ";
    }
    for (const [key, value] of Object.entries(options.env ?? {})) {
      args.push("-e", `${key}=${value}`);
    }
    let importCredsStep = "";
    if (options.credentials !== undefined && options.credentials.length > 0) {
      const credentialsPath = join(folder, "credentials.json");
      writeFileSync(credentialsPath, JSON.stringify(options.credentials), "utf8");
      chmodSync(credentialsPath, 0o644);
      importCredsStep = "n8n import:credentials --input=/tmp/bridge/credentials.json && ";
    }
    if (options.customExtensionsDir !== undefined) {
      args.push(
        "-e",
        "N8N_CUSTOM_EXTENSIONS=/tmp/n8n-custom",
        "-v",
        `${options.customExtensionsDir}:/tmp/n8n-custom:ro`,
      );
    }
    args.push(
      "-v",
      `${folder}:/tmp/bridge:rw`,
      "--entrypoint",
      "sh",
      image,
      "-c",
      `${startupPrefix}${importCredsStep}n8n import:workflow --input=/tmp/bridge/workflow.json && n8n execute --id="$1"`,
      "--",
      options.workflowId,
    );
    try {
      return await runCommand(engine, args, options.timeoutMs ?? 120_000);
    } catch (error) {
      await forceRemoveContainer(engine, containerName);
      throw error;
    }
  } finally {
    await closeServer(daemonBridge);
    rmSync(folder, { recursive: true, force: true });
  }
}

interface ActivepiecesSession {
  readonly token: string;
  readonly projectId: string;
}

function requestTimeoutMs(deadlineMs: number): number {
  const remaining = deadlineMs - Date.now();
  return Math.max(1, Math.min(remaining, PER_REQUEST_TIMEOUT_CAP_MS));
}

async function activepiecesFetch(
  url: string,
  init: RequestInit,
  context: string,
  deadlineMs: number,
): Promise<Response> {
  const response = await fetch(url, {
    ...init,
    signal: AbortSignal.timeout(requestTimeoutMs(deadlineMs)),
  });
  if (!response.ok) {
    throw new TestSupportError(
      "integration_harness_failed",
      `${context} failed (${response.status}): ${await response.text()}`,
    );
  }
  return response;
}

async function activepiecesJson(
  url: string,
  init: RequestInit,
  context: string,
  deadlineMs: number,
): Promise<Record<string, unknown>> {
  const response = await activepiecesFetch(url, init, context, deadlineMs);
  return await readJsonObject(response, context);
}

async function isContainerRunning(engine: ContainerEngine, containerId: string): Promise<boolean> {
  try {
    const inspected = await runCommand(
      engine,
      ["inspect", "-f", "{{.State.Running}}", containerId],
      5_000,
    );
    return inspected.exitCode === 0 && inspected.stdout.trim() === "true";
  } catch {
    return false;
  }
}

async function waitForActivepieces(
  engine: ContainerEngine,
  containerId: string,
  url: string,
  readyTimeoutMs: number,
): Promise<void> {
  const deadline = Date.now() + readyTimeoutMs;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${url}/api/v1/flags`, {
        signal: AbortSignal.timeout(Math.min(requestTimeoutMs(deadline), 3_000)),
      });
      if (response.ok) {
        return;
      }
    } catch {
      // Container still booting.
    }
    if (!(await isContainerRunning(engine, containerId))) {
      const logs = await runCommand(engine, ["logs", "--tail", "40", containerId], 5_000).catch(
        () => ({ stdout: "", stderr: "" }),
      );
      throw new TestSupportError(
        "integration_harness_failed",
        `Activepieces container ${containerId} exited before becoming ready: ${logs.stdout} ${logs.stderr}`.trim(),
      );
    }
    await delay(500);
  }
  const logs = await runCommand(engine, ["logs", "--tail", "40", containerId], 5_000).catch(() => ({
    stdout: "",
    stderr: "",
  }));
  throw new TestSupportError(
    "integration_harness_failed",
    `Activepieces did not become ready at ${url}: ${logs.stdout} ${logs.stderr}`.trim(),
  );
}

async function signUpActivepieces(url: string, deadlineMs: number): Promise<ActivepiecesSession> {
  const data = await activepiecesJson(
    `${url}/api/v1/authentication/sign-up`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        email: "test@goodfetch.test",
        password: "TestPassword123!",
        firstName: "Test",
        lastName: "User",
        trackEvents: false,
        newsLetter: false,
      }),
    },
    "Activepieces sign-up",
    deadlineMs,
  );
  return {
    token: readStringField(data, "token", "Activepieces sign-up"),
    projectId: readStringField(data, "projectId", "Activepieces sign-up"),
  };
}

async function runActivepiecesCodeStep(
  url: string,
  session: ActivepiecesSession,
  step: ActivepiecesCodeStep,
): Promise<ActivepiecesStepResult> {
  const deadlineMs = Date.now() + (step.timeoutMs ?? 60_000);
  const authHeaders = {
    Authorization: `Bearer ${session.token}`,
    "Content-Type": "application/json",
  };
  const flow = await activepiecesJson(
    `${url}/api/v1/flows`,
    {
      method: "POST",
      headers: authHeaders,
      body: JSON.stringify({ displayName: "Goodfetch Fixture Flow", projectId: session.projectId }),
    },
    "Activepieces create flow",
    deadlineMs,
  );
  const flowId = readStringField(flow, "id", "Activepieces create flow");
  const flowVersion = flow["version"];
  if (!isRecord(flowVersion)) {
    throw new TestSupportError(
      "integration_harness_failed",
      "Activepieces create flow: missing `version` object",
    );
  }
  const flowVersionId = readStringField(flowVersion, "id", "Activepieces flow version");
  await activepiecesFetch(
    `${url}/api/v1/flows/${flowId}`,
    {
      method: "POST",
      headers: authHeaders,
      body: JSON.stringify({
        type: "ADD_ACTION",
        request: {
          parentStep: "trigger",
          action: {
            name: "step_1",
            valid: true,
            displayName: "Call Daemon",
            type: "CODE",
            settings: {
              input: step.input,
              sourceCode: { code: step.code, packageJson: "{}" },
              errorHandlingOptions: {
                continueOnFailure: { value: false },
                retryOnFailure: { value: false },
              },
            },
          },
        },
      }),
    },
    "Activepieces ADD_ACTION",
    deadlineMs,
  );
  const queued = await activepiecesJson(
    `${url}/api/v1/sample-data/test-step`,
    {
      method: "POST",
      headers: authHeaders,
      body: JSON.stringify({
        projectId: session.projectId,
        flowId,
        flowVersionId,
        stepName: "step_1",
      }),
    },
    "Activepieces test-step",
    deadlineMs,
  );
  const runId = readStringField(queued, "id", "Activepieces test-step");
  while (Date.now() < deadlineMs) {
    const run = await activepiecesJson(
      `${url}/api/v1/flow-runs/${runId}`,
      { headers: { Authorization: `Bearer ${session.token}` } },
      "Activepieces flow-run poll",
      deadlineMs,
    );
    const status = readStringField(run, "status", "Activepieces flow-run poll");
    if (status !== "QUEUED" && status !== "RUNNING") {
      const steps = run["steps"];
      const stepOne = isRecord(steps) ? steps["step_1"] : undefined;
      let output: unknown = isRecord(stepOne) ? stepOne["output"] : undefined;
      if (output === undefined && status === "SUCCEEDED") {
        const sample = await fetch(
          `${url}/api/v1/sample-data?flowId=${encodeURIComponent(flowId)}&stepName=step_1&projectId=${encodeURIComponent(session.projectId)}&type=OUTPUT`,
          {
            headers: { Authorization: `Bearer ${session.token}` },
            signal: AbortSignal.timeout(requestTimeoutMs(deadlineMs)),
          },
        );
        if (sample.ok) {
          output = await sample.json();
        }
      }
      return { status, output, failedStep: run["failedStep"] };
    }
    await delay(300);
  }
  throw new TestSupportError(
    "integration_harness_failed",
    `Activepieces flow run ${runId} did not finish within ${step.timeoutMs ?? 60_000}ms`,
  );
}

/**
 * Starts the pinned Activepieces image as one container (`--network=none`)
 * with an embedded SQLite database (`AP_DB_TYPE=SQLITE3`), in-memory queue
 * (`AP_QUEUE_MODE=MEMORY`), and offline piece mode (`AP_PIECES_SYNC_MODE=NONE`).
 */
export async function startActivepieces(
  options: ActivepiecesStartOptions = {},
): Promise<StartedActivepieces> {
  const engine = requireEngine(options.engine);
  const image = options.image ?? readIntegrationImages().activepieces.reference;
  requireLocalImage(engine, image);
  const folder = mkdtempSync(join(tmpdir(), "goodfetch-ap-"));
  chmodSync(folder, 0o777);
  const bridgeScriptPath = join(folder, "bridge.cjs");
  writeFileSync(bridgeScriptPath, CONTAINER_BRIDGE_SCRIPT, "utf8");
  chmodSync(bridgeScriptPath, 0o644);
  // Activepieces 0.94.0's `code-builder.js` injects `@types/node@18.17.1` and
  // invokes `bun install` even for zero-dependency CODE steps, which fails
  // under `--network=none`; skipping `bun install` while delegating `bun build`
  // to `/usr/local/bin/bun` keeps execution 100% offline.
  const binDir = join(folder, "bin");
  mkdirSync(binDir, { mode: 0o755 });
  const bunWrapper = join(binDir, "bun");
  writeFileSync(
    bunWrapper,
    '#!/bin/sh\nif [ "$1" = "install" ]; then exit 0; fi\nexec /usr/local/bin/bun "$@"\n',
    "utf8",
  );
  chmodSync(bunWrapper, 0o755);
  const apBridge = await startTcpToUnixBridge(join(folder, "ap.sock"));
  let daemonBridge: NetServer | undefined;
  const url = `http://127.0.0.1:${apBridge.port}`;
  // Visibly fake local-only test values for the throwaway SQLite container.
  const encryptionKey = "0".repeat(32);
  const jwtSecret = "goodfetch-test-activepieces-jwt";
  let containerId = "";
  const cleanup = async (): Promise<void> => {
    if (containerId.length > 0) {
      await forceRemoveContainer(engine, containerId);
    }
    await closeServer(apBridge.server);
    await closeServer(daemonBridge);
    rmSync(folder, { recursive: true, force: true });
  };
  try {
    const args: string[] = [
      "run",
      "-d",
      "--rm",
      "--network=none",
      "-v",
      `${folder}:/tmp/bridge:rw`,
      "-e",
      "AP_DB_TYPE=SQLITE3",
      "-e",
      "AP_QUEUE_MODE=MEMORY",
      "-e",
      "AP_EXECUTION_MODE=UNSANDBOXED",
      "-e",
      "AP_PIECES_SYNC_MODE=NONE",
      "-e",
      `AP_PORT=${ACTIVEPIECES_CONTAINER_PORT}`,
      "-e",
      `AP_FRONTEND_URL=http://127.0.0.1:${ACTIVEPIECES_CONTAINER_PORT}`,
      "-e",
      `AP_ENCRYPTION_KEY=${encryptionKey}`,
      "-e",
      `AP_JWT_SECRET=${jwtSecret}`,
      "-e",
      "AP_TELEMETRY_ENABLED=false",
      "-e",
      "AP_SOCK=/tmp/bridge/ap.sock",
    ];
    if (options.daemonHttp !== undefined) {
      const daemonPort = parseLoopbackPort(options.daemonHttp);
      daemonBridge = await startUnixToTcpBridge(join(folder, "daemon.sock"), daemonPort);
      args.push("-e", `DAEMON_PORT=${daemonPort}`, "-e", "DAEMON_SOCK=/tmp/bridge/daemon.sock");
    }
    args.push(
      "--entrypoint",
      "sh",
      image,
      "-c",
      'export PATH="/tmp/bridge/bin:$PATH"; node /tmp/bridge/bridge.cjs & exec ./docker-entrypoint.sh',
    );
    const start = await runCommand(engine, args, 30_000);
    if (start.exitCode !== 0) {
      throw new TestSupportError(
        "integration_harness_failed",
        `Activepieces container failed to start: ${start.stderr.trim()}`,
      );
    }
    containerId = start.stdout.trim();
    if (containerId.length === 0) {
      throw new TestSupportError(
        "integration_harness_failed",
        `${engine} run -d printed no container ID`,
      );
    }
    await waitForActivepieces(engine, containerId, url, options.readyTimeoutMs ?? 60_000);
  } catch (error) {
    await cleanup();
    throw error;
  }

  let cachedSession: ActivepiecesSession | undefined;
  const runCodeStep = async (step: ActivepiecesCodeStep): Promise<ActivepiecesStepResult> => {
    const deadlineMs = Date.now() + (step.timeoutMs ?? 60_000);
    cachedSession ??= await signUpActivepieces(url, deadlineMs);
    return await runActivepiecesCodeStep(url, cachedSession, step);
  };

  return { url, port: apBridge.port, containerId, runCodeStep, stop: cleanup };
}
