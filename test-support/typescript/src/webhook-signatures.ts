// The shared Standard Webhooks vectors (ADR 0017), read from
// `test-support/fixtures/webhook-signatures.json`, the one file Rust and
// Python read too, so the engine's `webhook_signing` and the SDKs'
// verification helpers sign and verify alike.
import type { ValidateFunction } from "ajv/dist/2020.js";
import { ajv, committedSchema, sharedFixture } from "./schemas.ts";

/** A message, the keys that sign it and the `webhook-signature` they give. */
export interface SignedVector {
  /** The case's name. */
  name: string;
  /** Where the vector is published, for one the specification publishes. */
  source: string | null;
  /** The signing keys, current first: `whsec_` or `whsk_`. */
  keys: string[];
  /** The endpoint ID a `v2a` key signs for. */
  endpoint: string | null;
  /** `webhook-id`. */
  id: string;
  /** `webhook-timestamp`, in Unix seconds. */
  timestamp: number;
  /** The body, as sent. */
  body: string;
  /** `webhook-signature`: each key's signature, in order. */
  signature: string;
}

/** What the reference verifier answers. */
export type VerificationOutcome =
  | "verified"
  | "missing_header"
  | "timestamp_invalid"
  | "timestamp_too_old"
  | "timestamp_too_new"
  | "no_matching_signature";

/** A delivery as received, the keys trusted and the verifier's answer. */
export interface VerificationCase {
  /** The case's name. */
  name: string;
  /** The trusted keys: `whsec_` or `whpk_`. */
  trusted: string[];
  /** The endpoint ID a `v2a` signature is made for. */
  endpoint: string | null;
  /** `webhook-id`, `null` when missing. */
  id: string | null;
  /** `webhook-timestamp` as received, `null` when missing. */
  timestamp: string | null;
  /** `webhook-signature`, `null` when missing. */
  signature: string | null;
  /** The body, as received. */
  body: string;
  /** The receiver's clock, in Unix seconds. */
  now: number;
  /** The verifier's answer. */
  outcome: VerificationOutcome;
  /** The run epoch a verified delivery's body gives, when the case reads it. */
  run_epoch?: number | undefined;
}

/** The committed vectors. */
export interface WebhookSignatures {
  /** What a signer sends. */
  signed: SignedVector[];
  /** What the verifier answers. */
  verifications: VerificationCase[];
}

const isWebhookSignatures: ValidateFunction<WebhookSignatures> = ajv.compile<WebhookSignatures>(
  committedSchema("webhook-signatures.schema.json"),
);

/**
 * The committed vectors: the specification's published `v1` and `v2a` ones,
 * Goodfetch's rotations, and the verification cases.
 */
export function webhookSignatures(): WebhookSignatures {
  return sharedFixture("webhook-signatures.json", isWebhookSignatures);
}
