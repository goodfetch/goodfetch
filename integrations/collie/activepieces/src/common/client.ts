// Shared Activepieces client and error wrapper over
// `@goodfetch/collie-integrations-core` (ADRs 0027, 0070, 0090, 0095; spec
// #608, issue 1409).
import {
  IntegrationHttpClient,
  IntegrationToolError,
  mapCatalogError,
} from "@goodfetch/collie-integrations-core";
import type { ActivepiecesCollieAuth, PieceAuthDefinition } from "../ap-types.ts";

export class ActivepiecesCollieError extends Error {
  readonly code: string;
  readonly group: string;
  readonly fix: string;
  readonly httpStatus: number | undefined;
  readonly details: Readonly<Record<string, unknown>> | undefined;

  constructor(init: {
    readonly code: string;
    readonly group: string;
    readonly fix: string;
    readonly message: string;
    readonly httpStatus?: number | undefined;
    readonly details?: Readonly<Record<string, unknown>> | undefined;
  }) {
    super(`[${init.code}] (${init.group}) ${init.message} Fix: ${init.fix}`);
    this.name = "ActivepiecesCollieError";
    this.code = init.code;
    this.group = init.group;
    this.fix = init.fix;
    this.httpStatus = init.httpStatus;
    this.details = init.details;
  }
}

export function toActivepiecesError(error: unknown): ActivepiecesCollieError {
  if (error instanceof ActivepiecesCollieError) {
    return error;
  }
  if (error instanceof IntegrationToolError) {
    return new ActivepiecesCollieError({
      code: error.code,
      group: error.group,
      fix: error.fix,
      message: error.message,
      httpStatus: error.httpStatus,
      details: error.details,
    });
  }
  const message = error instanceof Error ? error.message : String(error);
  return new ActivepiecesCollieError({
    code: "internal_error",
    group: "internal",
    fix: "Inspect the Collie Daemon logs and retry.",
    message,
  });
}

export function mapCatalogErrorToActivepieces(entry: {
  readonly code: string;
  readonly group: string;
  readonly description?: string | undefined;
}): ActivepiecesCollieError {
  const mapped = mapCatalogError(entry);
  return new ActivepiecesCollieError({
    code: mapped.code,
    group: mapped.group,
    fix: mapped.fix,
    message:
      entry.description !== undefined && entry.description.trim().length > 0
        ? entry.description.trim()
        : mapped.code,
  });
}

export function createCollieClient(
  auth: ActivepiecesCollieAuth,
  fetchImpl?: typeof fetch,
): IntegrationHttpClient {
  const baseUrl = auth.baseUrl.trim();
  if (baseUrl.length === 0) {
    throw new ActivepiecesCollieError({
      code: "invalid_credentials",
      group: "program",
      fix: "Set Base URL in your Goodfetch Collie connection settings.",
      message: "Goodfetch Collie connection requires a non-empty Base URL.",
    });
  }
  return new IntegrationHttpClient({
    baseUrl,
    token: auth.token,
    ...(auth.protocolVersion === undefined ? {} : { protocolVersion: auth.protocolVersion }),
    ...(fetchImpl === undefined ? {} : { fetchImpl }),
  });
}

export function parseOptionalObject(
  value: unknown,
  fieldName: string,
): Record<string, unknown> | undefined {
  if (value === undefined || value === null || value === "") {
    return undefined;
  }
  let parsed: unknown = value;
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (trimmed.length === 0) {
      return undefined;
    }
    try {
      parsed = JSON.parse(trimmed);
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      throw new ActivepiecesCollieError({
        code: "invalid_json_parameter",
        group: "program",
        fix: `Provide a valid JSON object for ${fieldName}.`,
        message: `Invalid JSON in ${fieldName}: ${detail}`,
      });
    }
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new ActivepiecesCollieError({
      code: "invalid_json_parameter",
      group: "program",
      fix: `Provide a JSON object for ${fieldName}.`,
      message: `${fieldName} must be a JSON object.`,
    });
  }
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(parsed)) {
    out[k] = v;
  }
  return out;
}

export const goodfetchCollieAuth: PieceAuthDefinition<ActivepiecesCollieAuth> = {
  type: "CUSTOM_AUTH",
  displayName: "Goodfetch Collie Connection",
  description: "Connect to a Collie Daemon HTTP JSON-RPC listener using its Bearer token.",
  required: true,
  props: {
    baseUrl: {
      displayName: "Base URL",
      description:
        "Base URL of the Collie Daemon HTTP listener (for example http://127.0.0.1:7801).",
      type: "SHORT_TEXT",
      required: true,
      defaultValue: "http://127.0.0.1:7801",
    },
    token: {
      displayName: "Bearer Token",
      description: "Bearer token configured on the Collie Daemon HTTP listener.",
      type: "SECRET_TEXT",
      required: true,
    },
    protocolVersion: {
      displayName: "Protocol Version",
      description: "Optional Goodfetch protocol version header (defaults to 0.1).",
      type: "SHORT_TEXT",
      required: false,
      defaultValue: "0.1",
    },
  },
  async validate({ auth }) {
    try {
      const client = createCollieClient(auth);
      await client.call("daemon.status", {});
      return { valid: true };
    } catch (error) {
      const apError = toActivepiecesError(error);
      return { valid: false, error: apError.message };
    }
  },
};
