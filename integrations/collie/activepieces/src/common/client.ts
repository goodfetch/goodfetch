// Shared Activepieces client, auth definition, and error wrapper over
// `@goodfetch/collie-integrations-core` using `@activepieces/pieces-framework`
// (ADRs 0027, 0070, 0090, 0095; spec #608, issue 1409).
import {
  PieceAuth,
  Property,
  type CustomAuthProperty,
  type SecretTextProperty,
  type ShortTextProperty,
} from "@activepieces/pieces-framework";
import {
  IntegrationHttpClient,
  IntegrationToolError,
  mapCatalogError,
} from "@goodfetch/collie-integrations-core";

export interface ActivepiecesCollieAuth {
  readonly baseUrl: string;
  readonly token: string;
  readonly protocolVersion?: string | undefined;
}

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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Extracts `ActivepiecesCollieAuth` from either Activepieces' runtime
 * `AppConnectionValue<CUSTOM_AUTH, ...>` (`{ props: { ... } }`) or direct
 * `validate({ auth })` props (`{ baseUrl, token, protocolVersion }`).
 */
export function extractCollieAuth(raw: unknown): ActivepiecesCollieAuth {
  const source = isRecord(raw) && isRecord(raw["props"]) ? raw["props"] : raw;
  if (!isRecord(source)) {
    throw new ActivepiecesCollieError({
      code: "invalid_credentials",
      group: "program",
      fix: "Configure Base URL and Bearer Token in your Goodfetch Collie connection.",
      message: "Goodfetch Collie connection auth is missing.",
    });
  }
  const baseUrl = typeof source["baseUrl"] === "string" ? source["baseUrl"] : "";
  const token = typeof source["token"] === "string" ? source["token"] : "";
  const protocolVersion =
    typeof source["protocolVersion"] === "string" && source["protocolVersion"].trim().length > 0
      ? source["protocolVersion"].trim()
      : undefined;
  return { baseUrl, token, protocolVersion };
}

export function createCollieClient(
  rawAuth: unknown,
  fetchImpl?: typeof fetch,
): IntegrationHttpClient {
  const auth = extractCollieAuth(rawAuth);
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
  if (!isRecord(parsed)) {
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

export function parseOptionalStringArray(value: unknown, fieldName: string): string[] | undefined {
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
        fix: `Provide a valid JSON array of strings for ${fieldName}.`,
        message: `Invalid JSON in ${fieldName}: ${detail}`,
      });
    }
  }
  if (!Array.isArray(parsed) || !parsed.every((item): item is string => typeof item === "string")) {
    throw new ActivepiecesCollieError({
      code: "invalid_json_parameter",
      group: "program",
      fix: `Provide a JSON array of strings for ${fieldName}.`,
      message: `${fieldName} must be a JSON array of strings.`,
    });
  }
  return [...parsed];
}

export type GoodfetchCollieAuthProps = {
  baseUrl: ShortTextProperty<true>;
  token: SecretTextProperty<true>;
  protocolVersion: ShortTextProperty<false>;
};

export const goodfetchCollieAuth: CustomAuthProperty<GoodfetchCollieAuthProps> =
  PieceAuth.CustomAuth({
    displayName: "Goodfetch Collie Connection",
    description: "Connect to a Collie Daemon HTTP JSON-RPC listener using its Bearer token.",
    required: true,
    props: {
      baseUrl: Property.ShortText({
        displayName: "Base URL",
        description:
          "Base URL of the Collie Daemon HTTP listener (for example http://127.0.0.1:7801).",
        required: true,
        defaultValue: "http://127.0.0.1:7801",
      }),
      token: PieceAuth.SecretText({
        displayName: "Bearer Token",
        description: "Bearer token configured on the Collie Daemon HTTP listener.",
        required: true,
      }),
      protocolVersion: Property.ShortText({
        displayName: "Protocol Version",
        description: "Optional Goodfetch protocol version header (defaults to 0.1).",
        required: false,
        defaultValue: "0.1",
      }),
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
  });
