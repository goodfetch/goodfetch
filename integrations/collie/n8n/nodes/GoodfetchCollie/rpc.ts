// Zero-dependency Collie HTTP JSON-RPC client, error mapper, and Standard
// Webhooks verifier for `@goodfetch/n8n-nodes-collie` (ADRs 0027, 0056, 0070,
// 0090, 0095; spec #608, issue 1405).
//
// Obeys n8n verified-node rules: zero runtime dependencies (uses only peer
// `n8n-workflow`), zero `process.env` or filesystem access, and Web Crypto
// (`crypto.subtle`) for HMAC-SHA256 webhook verification.
import {
  NodeOperationError,
  type IDataObject,
  type IHttpRequestOptions,
  type INode,
} from "n8n-workflow";

export const COLLIE_PRODUCT = "collie" as const;
export const COLLIE_FORM = "catalog" as const;
export const DEFAULT_PROTOCOL_VERSION = "0.1" as const;
export const RPC_PATH = "/v1/rpc" as const;

export type CollieErrorGroup =
  | "program"
  | "refused"
  | "page"
  | "model"
  | "deadline_exceeded"
  | "unavailable"
  | "internal";

const GROUP_FIX_GUIDANCE: Readonly<Record<CollieErrorGroup, string>> = {
  program: "Check the n8n node parameters, JSON inputs, or settings and try again.",
  refused: "Check the Collie Daemon policy, limits, token, or approval rules before retrying.",
  page: "Inspect the Run trace and adjust the Recipe, selector, wait, or access mode.",
  model: "Check the Model provider configuration or switch to another model.",
  deadline_exceeded: "Increase the timeout or wait budget and try again.",
  unavailable: "Check that the Collie Daemon and network dependencies are reachable and retry.",
  internal: "Inspect the Collie Daemon logs and report the internal error if it persists.",
};

const WIRE_CODE_TO_GROUP: Readonly<Record<number, CollieErrorGroup>> = {
  [-32700]: "program",
  [-32600]: "program",
  [-32601]: "program",
  [-32602]: "program",
  [-32603]: "internal",
  [-32001]: "program",
  [-32002]: "refused",
  [-32003]: "page",
  [-32004]: "model",
  [-32005]: "deadline_exceeded",
  [-32006]: "unavailable",
  [-32007]: "internal",
};

export interface CollieCredentials {
  readonly baseUrl: string;
  readonly token: string;
  readonly protocolVersion: string;
}

export interface N8nCollieErrorInfo {
  readonly code: string;
  readonly group: CollieErrorGroup;
  readonly fix: string;
  readonly message: string;
  readonly httpStatus?: number | undefined;
  readonly details?: IDataObject | undefined;
}

export interface N8nCollieErrorMetadata {
  readonly code: string;
  readonly group: CollieErrorGroup;
  readonly fix: string;
  readonly httpCode: string | undefined;
  readonly details: IDataObject | undefined;
}

export interface CollieHttpHelper {
  httpRequest(options: IHttpRequestOptions): Promise<unknown>;
}

const FALLBACK_NODE: INode = {
  id: "goodfetch-collie",
  name: "Goodfetch Collie",
  type: "goodfetchCollie",
  typeVersion: 1,
  position: [0, 0],
  parameters: {},
};

const ERROR_METADATA = new WeakMap<Error, N8nCollieErrorMetadata>();

export function readCollieErrorMetadata(error: unknown): N8nCollieErrorMetadata | undefined {
  return error instanceof Error ? ERROR_METADATA.get(error) : undefined;
}

export function createCollieNodeError(info: N8nCollieErrorInfo, node?: INode): NodeOperationError {
  const err = new NodeOperationError(
    node ?? FALLBACK_NODE,
    `[${info.code}] (${info.group}) ${info.message} Fix: ${info.fix}`,
    {
      description: `code=${info.code} group=${info.group} fix=${info.fix}`,
    },
  );
  ERROR_METADATA.set(err, {
    code: info.code,
    group: info.group,
    fix: info.fix,
    httpCode: info.httpStatus === undefined ? undefined : String(info.httpStatus),
    details: info.details,
  });
  return err;
}

export function isRecord(value: unknown): value is IDataObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function readStringParam(raw: unknown, fallback = ""): string {
  return typeof raw === "string" ? raw : fallback;
}

export function normalizeGroup(raw: unknown): CollieErrorGroup {
  switch (raw) {
    case "program":
    case "refused":
    case "page":
    case "model":
    case "deadline_exceeded":
    case "unavailable":
    case "internal":
      return raw;
    default:
      return "internal";
  }
}

export function fixForCollieError(code: string, group: CollieErrorGroup, message?: string): string {
  if (code === "unauthorized" || code === "token_invalid") {
    return "Verify the Collie Daemon HTTP listener Bearer token in your n8n credentials.";
  }
  if (code === "protocol_version_unsupported") {
    return "Upgrade `@goodfetch/n8n-nodes-collie` or the Collie Daemon so their protocol versions match.";
  }
  if (code === "cdp_url_not_allowed") {
    return "Remove the loopback cdp_url; n8n nodes act for a model and cannot attach to local browsers.";
  }
  if (code === "setting_locked") {
    return "Remove locked settings (model keys, Secret bindings, prices) from the n8n node parameters.";
  }
  if (code === "egress_refused") {
    return "Use a publicly reachable webhook or target URL allowed by the Daemon's egress policy.";
  }
  if (message !== undefined && message.trim().length > 0) {
    return `${GROUP_FIX_GUIDANCE[group]} (${code}: ${message.trim()})`;
  }
  return GROUP_FIX_GUIDANCE[group];
}

export function mapCatalogErrorToN8n(
  entry: {
    readonly code: string;
    readonly group: string;
    readonly description?: string | undefined;
  },
  node?: INode,
): NodeOperationError {
  const group = normalizeGroup(entry.group);
  const message =
    entry.description !== undefined && entry.description.trim().length > 0
      ? entry.description.trim()
      : entry.code;
  return createCollieNodeError(
    {
      code: entry.code,
      group,
      fix: fixForCollieError(entry.code, group, entry.description),
      message,
    },
    node,
  );
}

export function parseCredentials(raw: IDataObject, node?: INode): CollieCredentials {
  const rawBaseUrl = typeof raw["baseUrl"] === "string" ? raw["baseUrl"].trim() : "";
  const rawToken = typeof raw["token"] === "string" ? raw["token"] : "";
  const rawProtocol =
    typeof raw["protocolVersion"] === "string" && raw["protocolVersion"].trim().length > 0
      ? raw["protocolVersion"].trim()
      : DEFAULT_PROTOCOL_VERSION;

  if (rawBaseUrl.length === 0) {
    throw createCollieNodeError(
      {
        code: "invalid_credentials",
        group: "program",
        fix: "Set Base URL in your Goodfetch Collie API credentials (for example http://127.0.0.1:7801).",
        message: "Goodfetch Collie API credentials require a non-empty Base URL.",
      },
      node,
    );
  }
  return {
    baseUrl: rawBaseUrl.replace(/\/+$/, ""),
    token: rawToken,
    protocolVersion: rawProtocol,
  };
}

export function parseJsonObjectParam(
  raw: unknown,
  fieldName: string,
  node?: INode,
): IDataObject | undefined {
  if (raw === undefined || raw === null || raw === "") {
    return undefined;
  }
  let parsed: unknown = raw;
  if (typeof raw === "string") {
    const trimmed = raw.trim();
    if (trimmed.length === 0) {
      return undefined;
    }
    try {
      parsed = JSON.parse(trimmed);
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      throw createCollieNodeError(
        {
          code: "invalid_json_parameter",
          group: "program",
          fix: `Provide a valid JSON object for ${fieldName}.`,
          message: `Invalid JSON in ${fieldName}: ${detail}`,
        },
        node,
      );
    }
  }
  if (!isRecord(parsed)) {
    throw createCollieNodeError(
      {
        code: "invalid_json_parameter",
        group: "program",
        fix: `Provide a JSON object (not an array or primitive) for ${fieldName}.`,
        message: `${fieldName} must be a JSON object.`,
      },
      node,
    );
  }
  return parsed;
}

export function parseJsonStringArrayParam(
  raw: unknown,
  fieldName: string,
  node?: INode,
): string[] | undefined {
  if (raw === undefined || raw === null) {
    return undefined;
  }
  let parsed: unknown = raw;
  if (typeof raw === "string") {
    const trimmed = raw.trim();
    if (trimmed.length === 0) {
      return undefined;
    }
    try {
      parsed = JSON.parse(trimmed);
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      throw createCollieNodeError(
        {
          code: "invalid_json_parameter",
          group: "program",
          fix: `Provide a valid JSON array of strings for ${fieldName}.`,
          message: `Invalid JSON in ${fieldName}: ${detail}`,
        },
        node,
      );
    }
  }
  if (!Array.isArray(parsed) || !parsed.every((item): item is string => typeof item === "string")) {
    throw createCollieNodeError(
      {
        code: "invalid_json_parameter",
        group: "program",
        fix: `Provide a JSON array of strings for ${fieldName}.`,
        message: `${fieldName} must be a JSON array of strings.`,
      },
      node,
    );
  }
  return [...parsed];
}

export function parseRpcErrorResponse(
  body: unknown,
  httpStatus: number,
  node?: INode,
): NodeOperationError {
  let parsedBody: unknown = body;
  if (typeof body === "string" && body.trim().length > 0) {
    try {
      parsedBody = JSON.parse(body);
    } catch {
      parsedBody = null;
    }
  }

  if (isRecord(parsedBody) && isRecord(parsedBody["error"])) {
    const rpcErr = parsedBody["error"];
    const data = isRecord(rpcErr["data"]) ? rpcErr["data"] : undefined;
    const wireCode = typeof rpcErr["code"] === "number" ? rpcErr["code"] : undefined;
    const rawCode =
      typeof data?.["code"] === "string"
        ? data["code"]
        : httpStatus === 401 || httpStatus === 403
          ? "unauthorized"
          : wireCode !== undefined
            ? `jsonrpc_${wireCode}`
            : "internal_error";
    const wireGroup = wireCode !== undefined ? WIRE_CODE_TO_GROUP[wireCode] : undefined;
    const group: CollieErrorGroup =
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
        : fixForCollieError(rawCode, group, message);
    const details = isRecord(data?.["details"]) ? data["details"] : data;
    return createCollieNodeError(
      {
        code: rawCode,
        group,
        fix: explicitFix,
        message,
        httpStatus,
        details,
      },
      node,
    );
  }

  const code = httpStatus === 401 || httpStatus === 403 ? "unauthorized" : "unavailable";
  const group: CollieErrorGroup =
    httpStatus === 401 || httpStatus === 403 ? "refused" : "unavailable";
  const message = `Collie Daemon HTTP listener returned status ${httpStatus}`;
  return createCollieNodeError(
    {
      code,
      group,
      fix: fixForCollieError(code, group, message),
      message,
      httpStatus,
    },
    node,
  );
}

export async function callCollieRpc(options: {
  readonly helpers: CollieHttpHelper;
  readonly credentials: CollieCredentials;
  readonly method: string;
  readonly params: IDataObject;
  readonly node?: INode | undefined;
}): Promise<IDataObject> {
  const rpcUrl = options.credentials.baseUrl.endsWith(RPC_PATH)
    ? options.credentials.baseUrl
    : `${options.credentials.baseUrl}${RPC_PATH}`;

  let rawResponse: unknown;
  try {
    rawResponse = await options.helpers.httpRequest({
      method: "POST",
      url: rpcUrl,
      headers: {
        Authorization: `Bearer ${options.credentials.token}`,
        "Content-Type": "application/json",
        "Goodfetch-Protocol": options.credentials.protocolVersion,
        "Goodfetch-Product": COLLIE_PRODUCT,
        "Goodfetch-Form": COLLIE_FORM,
      },
      body: {
        jsonrpc: "2.0",
        id: 1,
        method: options.method,
        params: options.params,
      },
      json: true,
      returnFullResponse: true,
      ignoreHttpStatusErrors: true,
    });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw createCollieNodeError(
      {
        code: "daemon_unreachable",
        group: "unavailable",
        fix: fixForCollieError("daemon_unreachable", "unavailable", message),
        message,
      },
      options.node,
    );
  }

  let statusCode = 200;
  let body: unknown = rawResponse;
  if (isRecord(rawResponse) && typeof rawResponse["statusCode"] === "number") {
    statusCode = rawResponse["statusCode"];
    body = rawResponse["body"];
  }
  if (typeof body === "string" && body.trim().length > 0) {
    try {
      body = JSON.parse(body);
    } catch {
      // Leave raw string so parseRpcErrorResponse reports status/error.
    }
  }

  if (statusCode < 200 || statusCode >= 300 || (isRecord(body) && body["error"] !== undefined)) {
    throw parseRpcErrorResponse(body, statusCode, options.node);
  }

  if (!isRecord(body) || !isRecord(body["result"])) {
    throw createCollieNodeError(
      {
        code: "engine_output_invalid",
        group: "internal",
        fix: fixForCollieError("engine_output_invalid", "internal"),
        message: "Collie Daemon HTTP listener returned a non-object JSON-RPC result",
        httpStatus: statusCode,
      },
      options.node,
    );
  }

  return body["result"];
}

function decodeBase64Bytes(input: string): Uint8Array<ArrayBuffer> {
  const binary = atob(input);
  const buffer = new ArrayBuffer(binary.length);
  const bytes = new Uint8Array(buffer);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

function encodeBase64Bytes(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary);
}

function constantTimeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) {
    return false;
  }
  let diff = 0;
  for (let i = 0; i < a.length; i += 1) {
    diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return diff === 0;
}

function readHeader(
  headers: Record<string, string | string[] | undefined>,
  name: string,
): string | undefined {
  const target = name.toLowerCase();
  for (const [key, value] of Object.entries(headers)) {
    if (key.toLowerCase() !== target) {
      continue;
    }
    if (typeof value === "string") {
      return value;
    }
    if (Array.isArray(value) && value.length > 0) {
      return value[0];
    }
  }
  return undefined;
}

export async function verifyStandardWebhookSignature(options: {
  readonly rawBody: string;
  readonly headers: Record<string, string | string[] | undefined>;
  readonly secret: string;
  readonly nowSeconds?: number | undefined;
  readonly toleranceSeconds?: number | undefined;
}): Promise<boolean> {
  const webhookId = readHeader(options.headers, "webhook-id");
  const webhookTimestamp = readHeader(options.headers, "webhook-timestamp");
  const webhookSignature = readHeader(options.headers, "webhook-signature");
  if (webhookId === undefined || webhookTimestamp === undefined || webhookSignature === undefined) {
    return false;
  }

  const parsedTimestamp = Number(webhookTimestamp);
  if (!Number.isFinite(parsedTimestamp)) {
    return false;
  }
  const nowSeconds = options.nowSeconds ?? Math.floor(Date.now() / 1000);
  const tolerance = options.toleranceSeconds ?? 300;
  if (Math.abs(nowSeconds - parsedTimestamp) > tolerance) {
    return false;
  }

  const secretBody = options.secret.startsWith("whsec_")
    ? options.secret.slice("whsec_".length)
    : options.secret;
  let keyBytes: Uint8Array<ArrayBuffer>;
  try {
    keyBytes = decodeBase64Bytes(secretBody);
  } catch {
    return false;
  }

  const encoder = new TextEncoder();
  const signedPayload = encoder.encode(`${webhookId}.${webhookTimestamp}.${options.rawBody}`);
  const cryptoKey = await crypto.subtle.importKey(
    "raw",
    keyBytes,
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const signatureBuffer = await crypto.subtle.sign("HMAC", cryptoKey, signedPayload);
  const expected = encodeBase64Bytes(new Uint8Array(signatureBuffer));

  const candidates = webhookSignature.trim().split(/\s+/);
  for (const candidate of candidates) {
    const parts = candidate.split(",");
    if (parts.length === 2 && parts[0] === "v1" && parts[1] !== undefined) {
      if (constantTimeEqual(parts[1], expected)) {
        return true;
      }
    }
  }
  return false;
}
