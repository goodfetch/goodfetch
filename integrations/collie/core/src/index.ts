// Shared generated Integration layer for Collie (ADRs 0027, 0056, 0070, 0095;
// spec #608, issue 1403).
//
// Announces form `catalog` and Product `collie` over the Daemon's HTTP
// listener (`POST /v1/rpc`), exposes the four Integration actions and the
// `notification` trigger, maps every catalog Error code to an
// `IntegrationToolError` with `code`, `group`, and `fix`, and verifies
// incoming Standard Webhooks deliveries through `@goodfetch/collie-client`'s
// `verifyWebhook`.
import {
  errorGroupOfProtocolFailure,
  errorGroupOfWireClass,
  verifyWebhook,
  type ErrorGroup,
  type NotificationPayload,
  type VerifyWebhookOptions,
  type WebhookHeaders,
} from "@goodfetch/collie-client";
import { KEPT_OFF_OPERATIONS } from "@goodfetch/collie-client/mcp-tools";

export const INTEGRATION_PRODUCT = "collie" as const;
export const INTEGRATION_FORM = "catalog" as const;
export const INTEGRATION_PROTOCOL_VERSION = "0.1" as const;
export const RPC_PATH = "/v1/rpc" as const;

export interface IntegrationActionSpec {
  readonly id: "start-recipe" | "start-catalog-run" | "get-run-report" | "get-run-progress";
  readonly operation: "recipe.start" | "catalog_run.start" | "run.report" | "run.progress";
  readonly detached: boolean;
  readonly description: string;
}

export interface IntegrationTriggerSpec {
  readonly id: "notification";
  readonly subscribeOperation: "notification.subscribe";
  readonly unsubscribeOperation: "notification.unsubscribe";
  readonly description: string;
}

export const INTEGRATION_ACTIONS: readonly IntegrationActionSpec[] = [
  {
    id: "start-recipe",
    operation: "recipe.start",
    detached: true,
    description: "Start a detached Recipe run on the Collie Daemon.",
  },
  {
    id: "start-catalog-run",
    operation: "catalog_run.start",
    detached: true,
    description: "Start a detached code-free catalog run on the Collie Daemon.",
  },
  {
    id: "get-run-report",
    operation: "run.report",
    detached: false,
    description: "Read the Run report for a Collie Run.",
  },
  {
    id: "get-run-progress",
    operation: "run.progress",
    detached: false,
    description: "Read the current progress for a Collie Run.",
  },
] as const;

export const INTEGRATION_TRIGGERS: readonly IntegrationTriggerSpec[] = [
  {
    id: "notification",
    subscribeOperation: "notification.subscribe",
    unsubscribeOperation: "notification.unsubscribe",
    description: "Fire when a Collie Run emits a selected Notification event.",
  },
] as const;

export const KEPT_OFF_INTEGRATION_OPERATIONS: readonly string[] = KEPT_OFF_OPERATIONS;

const GROUP_FIX_GUIDANCE: Readonly<Record<ErrorGroup, string>> = {
  program: "Check the Integration's action inputs, settings, or parameters and try again.",
  refused: "Check the Daemon policy, limits, token, or approval rules before retrying.",
  page: "Inspect the Run trace and adjust the Recipe, selector, wait, or access mode.",
  model: "Check the Model provider configuration or switch to another model.",
  deadline_exceeded: "Increase the timeout or wait budget and try again.",
  unavailable: "Check that the Collie Daemon and network dependencies are reachable and retry.",
  internal: "Inspect the Collie Daemon logs and report the internal error if it persists.",
};

const VALID_GROUPS: ReadonlySet<string> = new Set<ErrorGroup>([
  "program",
  "refused",
  "page",
  "model",
  "deadline_exceeded",
  "unavailable",
  "internal",
]);

function normalizeGroup(raw: unknown): ErrorGroup {
  if (typeof raw === "string" && VALID_GROUPS.has(raw)) {
    switch (raw) {
      case "program":
      case "refused":
      case "page":
      case "model":
      case "deadline_exceeded":
      case "unavailable":
      case "internal":
        return raw;
    }
  }
  return "internal";
}

export interface IntegrationErrorEntry {
  readonly code: string;
  readonly group: ErrorGroup;
  readonly fix: string;
}

export class IntegrationToolError extends Error {
  readonly code: string;
  readonly group: ErrorGroup;
  readonly fix: string;
  readonly details: Readonly<Record<string, unknown>> | undefined;
  readonly httpStatus: number | undefined;

  constructor(init: {
    readonly code: string;
    readonly group: ErrorGroup;
    readonly fix: string;
    readonly message: string;
    readonly details?: Readonly<Record<string, unknown>> | undefined;
    readonly httpStatus?: number | undefined;
  }) {
    super(`[${init.code}] (${init.group}) ${init.message} Fix: ${init.fix}`);
    this.name = "IntegrationToolError";
    this.code = init.code;
    this.group = init.group;
    this.fix = init.fix;
    this.details = init.details;
    this.httpStatus = init.httpStatus;
  }
}

export function fixForErrorCode(code: string, group: ErrorGroup, message?: string): string {
  if (code === "unauthorized" || code === "token_invalid") {
    return "Verify the Daemon HTTP listener Bearer token in the Integration credentials.";
  }
  if (code === "protocol_version_unsupported") {
    return "Upgrade the Collie Integration or Daemon so their protocol versions match.";
  }
  if (code === "cdp_url_not_allowed") {
    return "Remove the loopback cdp_url; Integrations act for a model and cannot attach to local browsers.";
  }
  if (code === "setting_locked") {
    return "Remove locked settings (model keys, Secret bindings, prices) from Integration run parameters.";
  }
  if (code === "egress_refused") {
    return "Use a publicly reachable webhook or target URL allowed by the Daemon's egress policy.";
  }
  if (message !== undefined && message.trim().length > 0) {
    return `${GROUP_FIX_GUIDANCE[group]} (${code}: ${message.trim()})`;
  }
  return GROUP_FIX_GUIDANCE[group];
}

export function mapCatalogError(entry: {
  readonly code: string;
  readonly group: string;
  readonly description?: string | undefined;
}): IntegrationErrorEntry {
  const group = normalizeGroup(entry.group);
  return {
    code: entry.code,
    group,
    fix: fixForErrorCode(entry.code, group, entry.description),
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function parseRpcError(body: unknown, httpStatus: number): IntegrationToolError {
  if (isRecord(body) && isRecord(body["error"])) {
    const rpcErr = body["error"];
    const data = isRecord(rpcErr["data"]) ? rpcErr["data"] : undefined;
    const rawCode =
      typeof data?.["code"] === "string"
        ? data["code"]
        : httpStatus === 401 || httpStatus === 403
          ? "unauthorized"
          : typeof rpcErr["code"] === "number"
            ? `jsonrpc_${rpcErr["code"]}`
            : "internal_error";
    const wireCode = typeof rpcErr["code"] === "number" ? rpcErr["code"] : undefined;
    const wireGroup =
      wireCode !== undefined
        ? (errorGroupOfWireClass[wireCode] ?? errorGroupOfProtocolFailure[wireCode])
        : undefined;
    const group =
      data?.["group"] !== undefined
        ? normalizeGroup(data["group"])
        : httpStatus === 401 || httpStatus === 403
          ? "refused"
          : wireGroup !== undefined
            ? wireGroup
            : "program";
    const message =
      typeof rpcErr["message"] === "string" ? rpcErr["message"] : `HTTP ${httpStatus} RPC error`;
    const explicitFix =
      typeof data?.["fix"] === "string" && data["fix"].trim().length > 0
        ? data["fix"]
        : fixForErrorCode(rawCode, group, message);
    const details = isRecord(data?.["details"]) ? data["details"] : data;
    return new IntegrationToolError({
      code: rawCode,
      group,
      fix: explicitFix,
      message,
      details,
      httpStatus,
    });
  }

  const code = httpStatus === 401 || httpStatus === 403 ? "unauthorized" : "unavailable";
  const group: ErrorGroup = httpStatus === 401 || httpStatus === 403 ? "refused" : "unavailable";
  const message = `Daemon HTTP listener returned status ${httpStatus}`;
  return new IntegrationToolError({
    code,
    group,
    fix: fixForErrorCode(code, group, message),
    message,
    httpStatus,
  });
}

export interface IntegrationClientOptions {
  readonly baseUrl: string;
  readonly token: string;
  readonly protocolVersion?: string | undefined;
  readonly fetchImpl?: typeof fetch | undefined;
}

export interface SubscribeNotificationResult {
  readonly channel: string;
  readonly created: boolean;
  readonly secret?: string | undefined;
}

export interface UnsubscribeNotificationResult {
  readonly channel: string;
  readonly removed: boolean;
}

export class IntegrationHttpClient {
  private readonly rpcUrl: string;
  private readonly token: string;
  private readonly protocolVersion: string;
  private readonly fetchImpl: typeof fetch;
  private nextId = 1;

  constructor(options: IntegrationClientOptions) {
    const trimmed = options.baseUrl.replace(/\/+$/, "");
    this.rpcUrl = trimmed.endsWith(RPC_PATH) ? trimmed : `${trimmed}${RPC_PATH}`;
    this.token = options.token;
    this.protocolVersion = options.protocolVersion ?? INTEGRATION_PROTOCOL_VERSION;
    this.fetchImpl = options.fetchImpl ?? globalThis.fetch;
  }

  async call(
    method: string,
    params: Record<string, unknown> = {},
  ): Promise<Record<string, unknown>> {
    const id = this.nextId;
    this.nextId += 1;

    let response: Response;
    try {
      response = await this.fetchImpl(this.rpcUrl, {
        method: "POST",
        headers: {
          Authorization: `Bearer ${this.token}`,
          "Content-Type": "application/json",
          "Goodfetch-Protocol": this.protocolVersion,
          "Goodfetch-Product": INTEGRATION_PRODUCT,
          "Goodfetch-Form": INTEGRATION_FORM,
        },
        body: JSON.stringify({
          jsonrpc: "2.0",
          id,
          method,
          params,
        }),
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      throw new IntegrationToolError({
        code: "daemon_unreachable",
        group: "unavailable",
        fix: fixForErrorCode("daemon_unreachable", "unavailable", message),
        message,
      });
    }

    const text = await response.text();
    let body: unknown = null;
    if (text.trim().length > 0) {
      try {
        body = JSON.parse(text);
      } catch {
        body = null;
      }
    }

    if (!response.ok || (isRecord(body) && body["error"] !== undefined)) {
      throw parseRpcError(body, response.status);
    }

    if (!isRecord(body) || !isRecord(body["result"])) {
      throw new IntegrationToolError({
        code: "engine_output_invalid",
        group: "internal",
        fix: fixForErrorCode("engine_output_invalid", "internal"),
        message: "Daemon HTTP listener returned a non-object JSON-RPC result",
        httpStatus: response.status,
      });
    }

    return body["result"];
  }

  async startRecipe(params: {
    readonly recipe: string;
    readonly runId?: string | undefined;
    readonly tags?: readonly string[] | undefined;
    readonly settings?: Readonly<Record<string, unknown>> | undefined;
    readonly timeout?: number | null | undefined;
  }): Promise<Record<string, unknown>> {
    const payload: Record<string, unknown> = {
      recipe: params.recipe,
      detached: true,
    };
    if (params.runId !== undefined) {
      payload["run_id"] = params.runId;
    }
    if (params.tags !== undefined) {
      payload["tags"] = params.tags;
    }
    if (params.settings !== undefined) {
      payload["settings"] = params.settings;
    }
    if (params.timeout !== undefined) {
      payload["timeout"] = params.timeout;
    }
    return this.call("recipe.start", payload);
  }

  async startCatalogRun(params: {
    readonly target: Readonly<Record<string, unknown>>;
    readonly runId?: string | undefined;
    readonly tags?: readonly string[] | undefined;
    readonly settings?: Readonly<Record<string, unknown>> | undefined;
    readonly timeout?: number | null | undefined;
  }): Promise<Record<string, unknown>> {
    const payload: Record<string, unknown> = {
      target: params.target,
      detached: true,
    };
    if (params.runId !== undefined) {
      payload["run_id"] = params.runId;
    }
    if (params.tags !== undefined) {
      payload["tags"] = params.tags;
    }
    if (params.settings !== undefined) {
      payload["settings"] = params.settings;
    }
    if (params.timeout !== undefined) {
      payload["timeout"] = params.timeout;
    }
    return this.call("catalog_run.start", payload);
  }

  async getRunReport(runId: string): Promise<Record<string, unknown>> {
    return this.call("run.report", { run_id: runId });
  }

  async getRunProgress(runId: string): Promise<Record<string, unknown>> {
    return this.call("run.progress", { run_id: runId });
  }

  async subscribeNotification(params: {
    readonly channel: string;
    readonly url: string;
    readonly events?: readonly string[] | undefined;
  }): Promise<SubscribeNotificationResult> {
    const payload: Record<string, unknown> = {
      channel: params.channel,
      url: params.url,
    };
    if (params.events !== undefined) {
      payload["events"] = [...params.events];
    }
    const result = await this.call("notification.subscribe", payload);
    const channel = typeof result["channel"] === "string" ? result["channel"] : params.channel;
    const created = result["created"] === true;
    const secret = typeof result["secret"] === "string" ? result["secret"] : undefined;
    return secret === undefined ? { channel, created } : { channel, created, secret };
  }

  async unsubscribeNotification(channel: string): Promise<UnsubscribeNotificationResult> {
    const result = await this.call("notification.unsubscribe", { channel });
    return {
      channel: typeof result["channel"] === "string" ? result["channel"] : channel,
      removed: result["removed"] === true,
    };
  }
}

export async function verifyNotificationDelivery(
  body: string | Uint8Array,
  headers: WebhookHeaders,
  secret: string | readonly string[],
  options?: VerifyWebhookOptions,
): Promise<NotificationPayload> {
  return verifyWebhook(body, headers, secret, options);
}
