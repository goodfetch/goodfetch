// Pinned n8n and Activepieces Integration harnesses (ADR 0070; issue 426).
// Both images are pinned by multi-platform digest in
// `ci/integrations/images.toml` and pulled before ADR 0043's outbound-traffic
// deny by `ci/integrations/fetch.sh` (`just fetch`). Each runs offline
// against `goodfetch-test-env --daemon --http-listener` on loopback.
import { execFile, execFileSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
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
  /** Stops and removes the container. */
  stop(): Promise<void>;
}

/** Options for `startActivepieces`. */
export interface ActivepiecesStartOptions {
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

/** Returns `"docker"` or `"podman"` if one answers `info`, else `undefined`. */
export function detectContainerEngine(): ContainerEngine | undefined {
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

async function freeLoopbackPort(): Promise<number> {
  return await new Promise<number>((resolvePort, rejectPort) => {
    const server = createServer();
    server.once("error", rejectPort);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (address === null || typeof address === "string") {
        server.close(() => {
          rejectPort(new Error("could not bind a loopback port"));
        });
        return;
      }
      const { port } = address;
      server.close((closeError) => {
        if (closeError) {
          rejectPort(closeError);
        } else {
          resolvePort(port);
        }
      });
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
        const exitCode = typeof error.code === "number" ? error.code : 1;
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
        resolveRun({ exitCode, stdout, stderr });
      },
    );
  });
}

/** Imports and executes one n8n workflow offline inside the pinned n8n image. */
export async function runN8nWorkflow(options: N8nWorkflowOptions): Promise<N8nWorkflowResult> {
  const engine = requireEngine(options.engine);
  const image = options.image ?? readIntegrationImages().n8n.reference;
  requireLocalImage(engine, image);
  const brokerPort = await freeLoopbackPort();
  const folder = mkdtempSync(join(tmpdir(), "goodfetch-n8n-"));
  const workflowPath = join(folder, "workflow.json");
  try {
    writeFileSync(workflowPath, JSON.stringify(options.workflow), "utf8");
    chmodSync(workflowPath, 0o644);
    const args: string[] = [
      "run",
      "--rm",
      "--network=host",
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
      "-e",
      `N8N_RUNNERS_BROKER_PORT=${brokerPort}`,
    ];
    for (const [key, value] of Object.entries(options.env ?? {})) {
      args.push("-e", `${key}=${value}`);
    }
    args.push(
      "-v",
      `${workflowPath}:/tmp/workflow.json:ro`,
      "--entrypoint",
      "sh",
      image,
      "-c",
      'n8n import:workflow --input=/tmp/workflow.json && n8n execute --id="$1"',
      "--",
      options.workflowId,
    );
    return await runCommand(engine, args, options.timeoutMs ?? 120_000);
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
}

interface ActivepiecesSession {
  readonly token: string;
  readonly projectId: string;
}

/**
 * Starts the pinned Activepieces image as one container with an embedded
 * SQLite database (`AP_DB_TYPE=SQLITE3`), in-memory queue (`AP_QUEUE_MODE=MEMORY`),
 * and offline piece mode (`AP_PIECES_SYNC_MODE=NONE`).
 */
export async function startActivepieces(
  options: ActivepiecesStartOptions = {},
): Promise<StartedActivepieces> {
  const engine = requireEngine(options.engine);
  const image = options.image ?? readIntegrationImages().activepieces.reference;
  requireLocalImage(engine, image);
  const port = await freeLoopbackPort();
  const url = `http://127.0.0.1:${port}`;
  // Visibly fake local-only test values for the throwaway SQLite container.
  const encryptionKey = "0".repeat(32);
  const jwtSecret = "goodfetch-test-activepieces-jwt";
  const start = await runCommand(
    engine,
    [
      "run",
      "-d",
      "--rm",
      "--network=host",
      "-e",
      "AP_DB_TYPE=SQLITE3",
      "-e",
      "AP_QUEUE_MODE=MEMORY",
      "-e",
      "AP_EXECUTION_MODE=UNSANDBOXED",
      "-e",
      "AP_PIECES_SYNC_MODE=NONE",
      "-e",
      `AP_PORT=${port}`,
      "-e",
      `AP_FRONTEND_URL=${url}`,
      "-e",
      `AP_ENCRYPTION_KEY=${encryptionKey}`,
      "-e",
      `AP_JWT_SECRET=${jwtSecret}`,
      "-e",
      "AP_TELEMETRY_ENABLED=false",
      image,
    ],
    30_000,
  );
  if (start.exitCode !== 0) {
    throw new TestSupportError(
      "integration_harness_failed",
      `Activepieces container failed to start: ${start.stderr.trim()}`,
    );
  }
  const containerId = start.stdout.trim();
  const stop = async (): Promise<void> => {
    try {
      await runCommand(engine, ["rm", "-f", containerId], 15_000);
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      process.stderr.write(
        `goodfetch-test-support: failed to stop Activepieces container ${containerId}: ${detail}\n`,
      );
    }
  };

  const deadline = Date.now() + (options.readyTimeoutMs ?? 60_000);
  let ready = false;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${url}/api/v1/flags`);
      if (response.ok) {
        ready = true;
        break;
      }
    } catch {
      // Container still booting.
    }
    await delay(500);
  }
  if (!ready) {
    const logs = await runCommand(engine, ["logs", "--tail", "40", containerId], 5_000).catch(
      () => ({ stdout: "", stderr: "" }),
    );
    await stop();
    throw new TestSupportError(
      "integration_harness_failed",
      `Activepieces did not become ready at ${url}: ${logs.stdout} ${logs.stderr}`.trim(),
    );
  }

  let cachedSession: ActivepiecesSession | undefined;
  const ensureSession = async (): Promise<ActivepiecesSession> => {
    if (cachedSession !== undefined) {
      return cachedSession;
    }
    const response = await fetch(`${url}/api/v1/authentication/sign-up`, {
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
    });
    if (!response.ok) {
      throw new TestSupportError(
        "integration_harness_failed",
        `Activepieces sign-up failed (${response.status}): ${await response.text()}`,
      );
    }
    const data = await readJsonObject(response, "Activepieces sign-up");
    cachedSession = {
      token: readStringField(data, "token", "Activepieces sign-up"),
      projectId: readStringField(data, "projectId", "Activepieces sign-up"),
    };
    return cachedSession;
  };

  const runCodeStep = async (step: ActivepiecesCodeStep): Promise<ActivepiecesStepResult> => {
    const session = await ensureSession();
    const authHeaders = {
      Authorization: `Bearer ${session.token}`,
      "Content-Type": "application/json",
    };
    const flowResponse = await fetch(`${url}/api/v1/flows`, {
      method: "POST",
      headers: authHeaders,
      body: JSON.stringify({ displayName: "Goodfetch Fixture Flow", projectId: session.projectId }),
    });
    if (!flowResponse.ok) {
      throw new TestSupportError(
        "integration_harness_failed",
        `Activepieces create flow failed (${flowResponse.status}): ${await flowResponse.text()}`,
      );
    }
    const flow = await readJsonObject(flowResponse, "Activepieces create flow");
    const flowId = readStringField(flow, "id", "Activepieces create flow");
    const flowVersion = flow["version"];
    if (!isRecord(flowVersion)) {
      throw new TestSupportError(
        "integration_harness_failed",
        "Activepieces create flow: missing `version` object",
      );
    }
    const flowVersionId = readStringField(flowVersion, "id", "Activepieces flow version");
    const addActionResponse = await fetch(`${url}/api/v1/flows/${flowId}`, {
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
    });
    if (!addActionResponse.ok) {
      throw new TestSupportError(
        "integration_harness_failed",
        `Activepieces ADD_ACTION failed (${addActionResponse.status}): ${await addActionResponse.text()}`,
      );
    }
    const runResponse = await fetch(`${url}/api/v1/sample-data/test-step`, {
      method: "POST",
      headers: authHeaders,
      body: JSON.stringify({
        projectId: session.projectId,
        flowId,
        flowVersionId,
        stepName: "step_1",
      }),
    });
    if (!runResponse.ok) {
      throw new TestSupportError(
        "integration_harness_failed",
        `Activepieces test-step failed (${runResponse.status}): ${await runResponse.text()}`,
      );
    }
    const queued = await readJsonObject(runResponse, "Activepieces test-step");
    const runId = readStringField(queued, "id", "Activepieces test-step");
    const pollDeadline = Date.now() + (step.timeoutMs ?? 60_000);
    while (Date.now() < pollDeadline) {
      const poll = await fetch(`${url}/api/v1/flow-runs/${runId}`, {
        headers: { Authorization: `Bearer ${session.token}` },
      });
      if (!poll.ok) {
        throw new TestSupportError(
          "integration_harness_failed",
          `Activepieces flow-run poll failed (${poll.status}): ${await poll.text()}`,
        );
      }
      const run = await readJsonObject(poll, "Activepieces flow-run poll");
      const status = readStringField(run, "status", "Activepieces flow-run poll");
      if (status !== "QUEUED" && status !== "RUNNING") {
        const steps = run["steps"];
        const stepOne = isRecord(steps) ? steps["step_1"] : undefined;
        let output: unknown = isRecord(stepOne) ? stepOne["output"] : undefined;
        if (output === undefined && status === "SUCCEEDED") {
          const sample = await fetch(
            `${url}/api/v1/sample-data?flowId=${encodeURIComponent(flowId)}&stepName=step_1&projectId=${encodeURIComponent(session.projectId)}&type=OUTPUT`,
            { headers: { Authorization: `Bearer ${session.token}` } },
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
  };

  return { url, port, containerId, runCodeStep, stop };
}
