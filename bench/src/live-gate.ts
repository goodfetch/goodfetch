// Shared live AI job gate (ADRs 0065 and 0076, ticket #413):
// - Disabled by repository variable until M4: exits at step 1 with zero calls.
// - Prints planned trials, models and worst-case USD from the committed price
//   table (`bench/live-ai-caps.json`) before the first call.
// - Refuses to start when worst-case USD exceeds the job's per-run cap.
// - Tracks a currency Limit during execution and stops when reached.
// - Classifies Vercel AI Gateway 429 rate limits and 402 exhausted free
//   credits as inconclusive (exit 75, matching `ci/live-checks.toml`).
// - Provides isolated Claude Code and Codex environments with telemetry,
//   error reporting and update checks turned off.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

/** Exit code `xtask results live-run` reads as inconclusive (ADR 0064). */
export const INCONCLUSIVE_EXIT_CODE = 75;

/** Every live AI job governed by ADR 0065's cap table. */
export type LiveJobId =
  | "provider-check"
  | "recorder"
  | "eval-pr"
  | "eval-nightly"
  | "eval-weekly"
  | "skills";

const LIVE_JOB_IDS: readonly LiveJobId[] = [
  "provider-check",
  "recorder",
  "eval-pr",
  "eval-nightly",
  "eval-weekly",
  "skills",
];

/** Whether `value` is a known live AI job identifier. */
export function isLiveJobId(value: string): value is LiveJobId {
  return LIVE_JOB_IDS.some((id) => id === value);
}

/** Per-job cap and repository enablement variable. */
export interface LiveJobConfig {
  readonly capUsd: number;
  readonly enabledVariable: string;
}

/** Price table entry for a pinned model on the gateway. */
export interface ModelPrice {
  readonly inputPerMillionUsd: number;
  readonly outputPerMillionUsd: number;
  readonly freeTier: boolean;
}

/** Committed cap and model price table (`bench/live-ai-caps.json`). */
export interface LiveCapsTable {
  readonly gateway: string;
  readonly apiKeyEnv: string;
  readonly jobs: Readonly<Record<LiveJobId, LiveJobConfig>>;
  readonly models: Readonly<Record<string, ModelPrice>>;
}

/** One batch of planned model calls for worst-case estimation. */
export interface PlannedTrial {
  readonly model: string;
  readonly count: number;
  readonly maxInputTokens: number;
  readonly maxOutputTokens: number;
}

/** Worst-case spend estimate computed before the first model call. */
export interface WorstCaseEstimate {
  readonly job: LiveJobId;
  readonly totalTrials: number;
  readonly models: readonly string[];
  readonly worstCaseUsd: number;
  readonly capUsd: number;
  readonly exceedsCap: boolean;
  readonly nonFreeTierModels: readonly string[];
}

/** Gateway response fields inspected for rate-limit / exhausted-credit signals. */
export interface GatewayResponseSignal {
  readonly status: number;
  readonly errorCode?: string;
  readonly body?: string;
  readonly retryAfterSeconds?: number | null;
}

/** Classification of a Vercel AI Gateway response. */
export type GatewayOutcome =
  | { readonly kind: "ok"; readonly status: number }
  | {
      readonly kind: "inconclusive";
      readonly reason: "rate_limit" | "exhausted_credit";
      readonly status: number;
      readonly retryAfterSeconds: number | null;
    }
  | { readonly kind: "error"; readonly status: number; readonly message: string };

/** Whether a live job is enabled by its repository variable. */
export interface JobEnablement {
  readonly enabled: boolean;
  readonly variable: string;
  readonly reason: string;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readPositiveNumber(record: Record<string, unknown>, key: string, context: string): number {
  const value = record[key];
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    throw new Error(`${context}.${key} must be a non-negative finite number`);
  }
  return value;
}

function readNonEmptyString(record: Record<string, unknown>, key: string, context: string): string {
  const value = record[key];
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${context}.${key} must be a non-empty string`);
  }
  return value;
}

function parseJobConfig(jobs: Record<string, unknown>, job: LiveJobId): LiveJobConfig {
  const entry = jobs[job];
  if (!isRecord(entry)) {
    throw new Error(`caps.jobs.${job} must be an object`);
  }
  return {
    capUsd: readPositiveNumber(entry, "capUsd", `caps.jobs.${job}`),
    enabledVariable: readNonEmptyString(entry, "enabledVariable", `caps.jobs.${job}`),
  };
}

function parseModels(raw: unknown): Readonly<Record<string, ModelPrice>> {
  if (!isRecord(raw)) {
    throw new Error("caps.models must be an object");
  }
  const models: Record<string, ModelPrice> = {};
  for (const [name, entry] of Object.entries(raw)) {
    if (!isRecord(entry)) {
      throw new Error(`caps.models[${name}] must be an object`);
    }
    const freeTier = entry["freeTier"];
    if (typeof freeTier !== "boolean") {
      throw new Error(`caps.models[${name}].freeTier must be a boolean`);
    }
    models[name] = {
      inputPerMillionUsd: readPositiveNumber(entry, "inputPerMillionUsd", `caps.models[${name}]`),
      outputPerMillionUsd: readPositiveNumber(entry, "outputPerMillionUsd", `caps.models[${name}]`),
      freeTier,
    };
  }
  return models;
}

/** Loads and validates `bench/live-ai-caps.json`. */
export function loadCapsTable(
  path: string = fileURLToPath(new URL("../live-ai-caps.json", import.meta.url)),
): LiveCapsTable {
  const parsed: unknown = JSON.parse(readFileSync(path, "utf8"));
  if (!isRecord(parsed)) {
    throw new Error(`${path} must be a JSON object`);
  }
  const jobsRaw = parsed["jobs"];
  if (!isRecord(jobsRaw)) {
    throw new Error(`${path}: jobs must be an object`);
  }
  return {
    gateway: readNonEmptyString(parsed, "gateway", "caps"),
    apiKeyEnv: readNonEmptyString(parsed, "apiKeyEnv", "caps"),
    jobs: {
      "provider-check": parseJobConfig(jobsRaw, "provider-check"),
      recorder: parseJobConfig(jobsRaw, "recorder"),
      "eval-pr": parseJobConfig(jobsRaw, "eval-pr"),
      "eval-nightly": parseJobConfig(jobsRaw, "eval-nightly"),
      "eval-weekly": parseJobConfig(jobsRaw, "eval-weekly"),
      skills: parseJobConfig(jobsRaw, "skills"),
    },
    models: parseModels(parsed["models"]),
  };
}

/** Checks whether a live AI job's repository variable enables it. */
export function isJobEnabled(
  job: LiveJobId,
  env: Readonly<Record<string, string | undefined>>,
  caps: LiveCapsTable = loadCapsTable(),
): JobEnablement {
  const variable = caps.jobs[job].enabledVariable;
  const raw = env[variable]?.trim().toLowerCase();
  if (raw === "1" || raw === "true" || raw === "yes") {
    return { enabled: true, variable, reason: `enabled by ${variable}=${raw}` };
  }
  return {
    enabled: false,
    variable,
    reason: `disabled: repository variable ${variable} is unset`,
  };
}

/** Computes worst-case spend from the committed price table before any call. */
export function estimateWorstCaseUsd(
  job: LiveJobId,
  trials: readonly PlannedTrial[],
  caps: LiveCapsTable = loadCapsTable(),
): WorstCaseEstimate {
  const capUsd = caps.jobs[job].capUsd;
  let worstCaseUsd = 0;
  let totalTrials = 0;
  const modelSet = new Set<string>();
  const nonFreeSet = new Set<string>();

  for (const trial of trials) {
    if (!Number.isInteger(trial.count) || trial.count < 0) {
      throw new Error(`planned trial count must be a non-negative integer, not ${trial.count}`);
    }
    if (trial.maxInputTokens < 0 || trial.maxOutputTokens < 0) {
      throw new Error(`planned token counts must be non-negative for ${trial.model}`);
    }
    const price = caps.models[trial.model];
    if (price === undefined) {
      throw new Error(`model "${trial.model}" is missing from the committed price table`);
    }
    modelSet.add(trial.model);
    if (!price.freeTier) {
      nonFreeSet.add(trial.model);
    }
    totalTrials += trial.count;
    const perTrial =
      (trial.maxInputTokens * price.inputPerMillionUsd +
        trial.maxOutputTokens * price.outputPerMillionUsd) /
      1_000_000;
    worstCaseUsd += perTrial * trial.count;
  }

  return {
    job,
    totalTrials,
    models: [...modelSet],
    worstCaseUsd,
    capUsd,
    exceedsCap: worstCaseUsd > capUsd,
    nonFreeTierModels: [...nonFreeSet],
  };
}

/** Formats the pre-call plan and worst-case summary printed before call #1. */
export function formatRunPlan(estimate: WorstCaseEstimate): string {
  return [
    `live-gate[${estimate.job}]: planned trials=${estimate.totalTrials}`,
    `models=[${estimate.models.join(", ")}]`,
    `worst case=$${estimate.worstCaseUsd.toFixed(4)} (cap=$${estimate.capUsd.toFixed(2)})`,
  ].join(" ");
}

/** Monotonic currency Limit tracker that stops execution once the cap is hit. */
export class CurrencyLimitTracker {
  readonly limitUsd: number;
  private spent = 0;
  private calls = 0;

  constructor(limitUsd: number) {
    if (!Number.isFinite(limitUsd) || limitUsd < 0) {
      throw new Error(`currency Limit must be a non-negative finite number, not ${limitUsd}`);
    }
    this.limitUsd = limitUsd;
  }

  get spentUsd(): number {
    return this.spent;
  }

  get remainingUsd(): number {
    return Math.max(0, this.limitUsd - this.spent);
  }

  get callsMade(): number {
    return this.calls;
  }

  /** Whether spending `amountUsd` stays within the currency Limit. */
  canAfford(amountUsd: number): boolean {
    if (!Number.isFinite(amountUsd) || amountUsd < 0) {
      throw new Error(`spend amount must be non-negative finite, not ${amountUsd}`);
    }
    return this.spent + amountUsd <= this.limitUsd + 1e-12;
  }

  /** Records a completed or committed spend; returns false without adding if over Limit. */
  recordSpend(amountUsd: number): boolean {
    if (!this.canAfford(amountUsd)) {
      return false;
    }
    this.spent += amountUsd;
    this.calls += 1;
    return true;
  }
}

const EXHAUSTED_CREDIT_PATTERN =
  /quota_for_entity_exceeded|insufficient[_\s-]?credits?|credit[s]?\s+exhausted|free[_\s-]?tier.*exhausted/i;

/** Classifies a Vercel AI Gateway response per ADRs 0064 and 0076. */
export function classifyGatewayResponse(response: GatewayResponseSignal): GatewayOutcome {
  if (response.status === 429) {
    return {
      kind: "inconclusive",
      reason: "rate_limit",
      status: 429,
      retryAfterSeconds: response.retryAfterSeconds ?? null,
    };
  }
  const combined = `${response.errorCode ?? ""} ${response.body ?? ""}`;
  if (response.status === 402 || EXHAUSTED_CREDIT_PATTERN.test(combined)) {
    return {
      kind: "inconclusive",
      reason: "exhausted_credit",
      status: response.status,
      retryAfterSeconds: response.retryAfterSeconds ?? null,
    };
  }
  if (response.status >= 200 && response.status < 300) {
    return { kind: "ok", status: response.status };
  }
  return {
    kind: "error",
    status: response.status,
    message: response.body ?? response.errorCode ?? `HTTP ${response.status}`,
  };
}

class LimitStoppedSignal extends Error {
  constructor() {
    super("currency limit reached");
  }
}

class GatewayInconclusiveSignal extends Error {
  readonly reason: "rate_limit" | "exhausted_credit";
  readonly status: number;

  constructor(reason: "rate_limit" | "exhausted_credit", status: number) {
    super(`gateway returned inconclusive signal (${reason}, HTTP ${status})`);
    this.reason = reason;
    this.status = status;
  }
}

/** Execution context passed to a live job's callback after all pre-call gates pass. */
export interface LiveJobContext {
  readonly job: LiveJobId;
  readonly capUsd: number;
  readonly gateway: string;
  readonly apiKey: string;
  readonly estimate: WorstCaseEstimate;
  canAfford(amountUsd: number): boolean;
  recordCall(actualUsd: number): void;
  stopAtLimit(): never;
  inconclusiveFromGateway(response: GatewayResponseSignal): never;
}

/** Options for `runLiveJob`. */
export interface LiveJobRunOptions<T> {
  readonly job: LiveJobId;
  readonly trials: readonly PlannedTrial[];
  readonly env?: Readonly<Record<string, string | undefined>>;
  readonly caps?: LiveCapsTable;
  readonly requireFreeTier?: boolean;
  readonly writeLog?: (line: string) => void;
  readonly execute: (context: LiveJobContext) => Promise<T>;
}

/** Structured outcome of a live AI job run. */
export type LiveJobOutcome<T> =
  | {
      readonly status: "disabled";
      readonly exitCode: 0;
      readonly variable: string;
      readonly callsMade: 0;
      readonly spentUsd: 0;
    }
  | {
      readonly status: "refused_over_cap";
      readonly exitCode: 1;
      readonly estimate: WorstCaseEstimate;
      readonly callsMade: 0;
      readonly spentUsd: 0;
    }
  | {
      readonly status: "inconclusive";
      readonly exitCode: typeof INCONCLUSIVE_EXIT_CODE;
      readonly reason: string;
      readonly callsMade: number;
      readonly spentUsd: number;
    }
  | {
      readonly status: "stopped_at_limit";
      readonly exitCode: 1;
      readonly capUsd: number;
      readonly callsMade: number;
      readonly spentUsd: number;
    }
  | {
      readonly status: "completed";
      readonly exitCode: 0;
      readonly value: T;
      readonly estimate: WorstCaseEstimate;
      readonly callsMade: number;
      readonly spentUsd: number;
    };

/**
 * Runs a live AI job through ADR 0065/0076's shared gates:
 * 1. Exits cleanly with zero calls when the repository variable is unset.
 * 2. Reports inconclusive (exit 75) if a model is not in the free tier.
 * 3. Prints the planned trials, models and worst-case USD before call #1.
 * 4. Refuses to start if worst-case USD exceeds the job's per-run cap.
 * 5. Enforces the currency Limit and classifies 429/402 as inconclusive.
 */
export async function runLiveJob<T>(options: LiveJobRunOptions<T>): Promise<LiveJobOutcome<T>> {
  const caps = options.caps ?? loadCapsTable();
  const env = options.env ?? process.env;
  const writeLog = options.writeLog ?? ((line: string) => process.stdout.write(`${line}\n`));

  const enablement = isJobEnabled(options.job, env, caps);
  if (!enablement.enabled) {
    writeLog(`live-gate[${options.job}]: ${enablement.reason}; exiting with 0 calls`);
    return {
      status: "disabled",
      exitCode: 0,
      variable: enablement.variable,
      callsMade: 0,
      spentUsd: 0,
    };
  }

  const estimate = estimateWorstCaseUsd(options.job, options.trials, caps);
  writeLog(formatRunPlan(estimate));

  if ((options.requireFreeTier ?? true) && estimate.nonFreeTierModels.length > 0) {
    const reason = `models not in Vercel AI Gateway free tier: ${estimate.nonFreeTierModels.join(", ")}`;
    writeLog(`live-gate[${options.job}]: inconclusive (${reason})`);
    return {
      status: "inconclusive",
      exitCode: INCONCLUSIVE_EXIT_CODE,
      reason,
      callsMade: 0,
      spentUsd: 0,
    };
  }

  if (estimate.exceedsCap) {
    writeLog(
      `live-gate[${options.job}]: refused — worst case $${estimate.worstCaseUsd.toFixed(4)} exceeds cap $${estimate.capUsd.toFixed(2)}`,
    );
    return {
      status: "refused_over_cap",
      exitCode: 1,
      estimate,
      callsMade: 0,
      spentUsd: 0,
    };
  }

  const apiKey = env[caps.apiKeyEnv]?.trim() ?? "";
  if (apiKey.length === 0) {
    const reason = `missing ${caps.apiKeyEnv}`;
    writeLog(`live-gate[${options.job}]: inconclusive (${reason})`);
    return {
      status: "inconclusive",
      exitCode: INCONCLUSIVE_EXIT_CODE,
      reason,
      callsMade: 0,
      spentUsd: 0,
    };
  }

  const tracker = new CurrencyLimitTracker(estimate.capUsd);
  const context: LiveJobContext = {
    job: options.job,
    capUsd: estimate.capUsd,
    gateway: caps.gateway,
    apiKey,
    estimate,
    canAfford: (amountUsd) => tracker.canAfford(amountUsd),
    recordCall: (actualUsd) => {
      if (!tracker.recordSpend(actualUsd)) {
        throw new LimitStoppedSignal();
      }
    },
    stopAtLimit: () => {
      throw new LimitStoppedSignal();
    },
    inconclusiveFromGateway: (response) => {
      const classified = classifyGatewayResponse(response);
      if (classified.kind === "inconclusive") {
        throw new GatewayInconclusiveSignal(classified.reason, classified.status);
      }
      throw new Error(`unexpected gateway response HTTP ${response.status}`);
    },
  };

  try {
    const value = await options.execute(context);
    return {
      status: "completed",
      exitCode: 0,
      value,
      estimate,
      callsMade: tracker.callsMade,
      spentUsd: tracker.spentUsd,
    };
  } catch (error) {
    if (error instanceof LimitStoppedSignal) {
      writeLog(
        `live-gate[${options.job}]: stopped at currency Limit ($${tracker.spentUsd.toFixed(4)} / $${estimate.capUsd.toFixed(2)})`,
      );
      return {
        status: "stopped_at_limit",
        exitCode: 1,
        capUsd: estimate.capUsd,
        callsMade: tracker.callsMade,
        spentUsd: tracker.spentUsd,
      };
    }
    if (error instanceof GatewayInconclusiveSignal) {
      writeLog(`live-gate[${options.job}]: inconclusive (${error.reason}, HTTP ${error.status})`);
      return {
        status: "inconclusive",
        exitCode: INCONCLUSIVE_EXIT_CODE,
        reason: `${error.reason} (HTTP ${error.status})`,
        callsMade: tracker.callsMade,
        spentUsd: tracker.spentUsd,
      };
    }
    throw error;
  }
}

/**
 * Isolated environment for Claude Code runs (ADR 0065 & ADR 0043): disables
 * nonessential traffic, auto-updater, telemetry and error reporting, routes
 * model calls solely to the Vercel AI Gateway, and carries `maxBudgetUsd`.
 */
export function claudeCodeLiveEnv(options: {
  readonly apiKey: string;
  readonly gatewayBaseUrl?: string;
  readonly maxBudgetUsd: number;
}): Record<string, string> {
  const gateway = options.gatewayBaseUrl ?? "https://ai-gateway.vercel.sh";
  return {
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
    DISABLE_AUTOUPDATER: "1",
    DISABLE_BUG_COMMAND: "1",
    DISABLE_ERROR_REPORTING: "1",
    DISABLE_TELEMETRY: "1",
    AI_GATEWAY_API_KEY: options.apiKey,
    ANTHROPIC_BASE_URL: gateway,
    GOODFETCH_MAX_BUDGET_USD: String(options.maxBudgetUsd),
  };
}

/**
 * Isolated environment for Codex runs (ADR 0065 & ADR 0043): disables update
 * checks and analytics and routes calls solely to the Vercel AI Gateway.
 */
export function codexLiveEnv(options: {
  readonly apiKey: string;
  readonly gatewayBaseUrl?: string;
}): Record<string, string> {
  const gateway = options.gatewayBaseUrl ?? "https://ai-gateway.vercel.sh";
  const baseUrl = gateway.endsWith("/v1") ? gateway : `${gateway}/v1`;
  return {
    CODEX_DISABLE_UPDATE_CHECK: "1",
    CODEX_DISABLE_ANALYTICS: "1",
    DO_NOT_TRACK: "1",
    AI_GATEWAY_API_KEY: options.apiKey,
    OPENAI_BASE_URL: baseUrl,
  };
}

/** Generates a Codex `config.toml` with updates/analytics off and gateway routing. */
export function codexIsolatedConfigToml(gatewayV1BaseUrl: string): string {
  return [
    "check_for_update_on_startup = false",
    'model_provider = "vercel_ai_gateway"',
    "",
    "[analytics]",
    "enabled = false",
    "",
    "[model_providers.vercel_ai_gateway]",
    'name = "Vercel AI Gateway"',
    `base_url = "${gatewayV1BaseUrl}"`,
    'env_key = "AI_GATEWAY_API_KEY"',
    "",
  ].join("\n");
}
