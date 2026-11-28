// The caller as a Secret provider (ADRs 0067, 0092 and 0104): `connect({ resolveSecret })`
// answers `caller:<name>` Secret references at the moment of use, for Runs its
// own Connection opened. The value goes straight to the engine; nothing here
// keeps a copy, logs it or repeats a thrown error's message in the JSON-RPC
// reply, since a thrown error may hold a Secret value.
import type { Served } from "./client-connection.ts";
import { isRecord } from "./framing.ts";
import { SecretUnavailableError } from "./generated/errors.ts";

/** The handler request the engine sends for a `caller:<name>` Secret reference. */
export const SECRETS_RESOLVE = "secrets.resolve";

/** What the engine passes to `resolveSecret` for a `caller:<name>` reference (ADR 0092). */
export interface SecretRequest {
  /** The reference's name, what follows `caller:`. */
  readonly name: string;
  /** The Run that uses it, which this Connection opened. */
  readonly runId: string;
  /** The host the value is sent to, or `undefined` where the use has none. */
  readonly domain?: string | undefined;
}

/** Answers `caller:<name>` Secret references at the moment of use (ADR 0092). */
export type SecretResolver = (request: SecretRequest) => Promise<string>;

/**
 * Serves `secrets.resolve` through `resolveSecret`: passes the value straight
 * back as `{ value }` without keeping or logging it, and turns any throw or
 * non-string return into a `SecretUnavailableError` whose message and details
 * carry no text from the thrown error (ADR 0067).
 */
export function serveSecretResolver(resolveSecret: SecretResolver): Served {
  return async (params: unknown): Promise<{ readonly value: string }> => {
    const request = parseSecretRequest(params);
    let value: unknown;
    try {
      value = await resolveSecret(request);
    } catch (error) {
      throw redactedUnavailable(error);
    }
    if (typeof value !== "string") {
      throw new SecretUnavailableError("resolveSecret returned no string value", {
        details: { failure: "resolver_failed" },
      });
    }
    return { value };
  };
}

/** Reads `secrets.resolve`'s wire params `{ name, run_id, domain? }` as a `SecretRequest`. */
function parseSecretRequest(params: unknown): SecretRequest {
  if (!isRecord(params)) {
    throw new SecretUnavailableError("secrets.resolve params are not an object", {
      details: { failure: "resolver_failed" },
    });
  }
  const name = params["name"];
  const runId = params["run_id"];
  const domain = params["domain"];
  if (
    typeof name !== "string" ||
    typeof runId !== "string" ||
    (domain !== undefined && domain !== null && typeof domain !== "string")
  ) {
    throw new SecretUnavailableError("secrets.resolve params are malformed", {
      details: { failure: "resolver_failed" },
    });
  }
  return typeof domain === "string" ? { name, runId, domain } : { name, runId };
}

/** Closed set of wire `SecretFailure` tags (ADR 0067). */
const SECRET_FAILURES: ReadonlySet<string> = new Set([
  "missing",
  "not_configured",
  "provider_failed",
  "resolver_failed",
  "timed_out",
  "no_code",
]);

/**
 * Turns any error thrown by `resolveSecret` into a redacted
 * `SecretUnavailableError`, keeping only a known `failure` tag when the
 * caller raised `SecretUnavailableError` itself and never echoing the
 * exception's message, stack or cause (ADR 0067).
 */
function redactedUnavailable(error: unknown): SecretUnavailableError {
  const rawFailure = error instanceof SecretUnavailableError ? error.details["failure"] : undefined;
  const failure =
    typeof rawFailure === "string" && SECRET_FAILURES.has(rawFailure)
      ? rawFailure
      : "resolver_failed";
  return new SecretUnavailableError("resolveSecret failed", {
    details: { failure },
  });
}
