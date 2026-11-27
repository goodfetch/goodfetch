// The verification helper through its interface (issue 1391): the shared
// Standard Webhooks vectors, which the Rust and Python helpers pass too, and
// Notification payloads this test signs as the engine does. The Rust
// helper's `webhook/tests.rs` is its twin.
import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { ProgramError, WebhookInvalidError } from "./generated/errors.ts";
import type { NotificationPayload, WebhookInvalidReason } from "./generated/types.ts";
import { verifyWebhook, type VerifyWebhookOptions } from "./webhook.ts";

interface SignedVector {
  readonly name: string;
  readonly keys: readonly string[];
  readonly endpoint: string | null;
}

interface VerificationCase {
  readonly name: string;
  readonly trusted: readonly string[];
  readonly endpoint: string | null;
  readonly id: string | null;
  readonly timestamp: string | null;
  readonly signature: string | null;
  readonly body: string;
  readonly now: number;
  readonly outcome:
    | "verified"
    | "no_matching_signature"
    | "timestamp_too_old"
    | "timestamp_too_new"
    | "timestamp_invalid"
    | "missing_header";
}

// oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the committed vectors file lists signed vectors and verification cases of these shapes.
const vectors = JSON.parse(
  readFileSync(
    new URL("../../../../test-support/fixtures/webhook-signatures.json", import.meta.url),
    "utf8",
  ),
) as {
  readonly signed: readonly SignedVector[];
  readonly verifications: readonly VerificationCase[];
};

function signedVector(name: string): SignedVector {
  const vector = vectors.signed.find((each) => each.name === name);
  if (vector === undefined) {
    throw new Error(`no signed vector ${name}`);
  }
  return vector;
}

function verification(name: string): VerificationCase {
  const found = vectors.verifications.find((each) => each.name === name);
  if (found === undefined) {
    throw new Error(`no verification case ${name}`);
  }
  return found;
}

/** When the test deliveries are signed, in Unix seconds. */
const SIGNED_AT = 1_767_603_600;
const EVENT_ID = "evt_01JDQ8X2Y5";

/** `seconds` after the Unix epoch. */
function at(seconds: number): Date {
  return new Date(seconds * 1000);
}

function bytesOfBase64(text: string): Uint8Array<ArrayBuffer> {
  return Uint8Array.from(atob(text), (character) => character.charCodeAt(0));
}

function base64Of(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes));
}

function base64UrlOf(bytes: Uint8Array): string {
  return base64Of(bytes).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}

function concat(...parts: readonly Uint8Array[]): Uint8Array<ArrayBuffer> {
  const joined = new Uint8Array(parts.reduce((length, part) => length + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    joined.set(part, offset);
    offset += part.length;
  }
  return joined;
}

/** A `v1` signature of the message with the `whsec_` secret `secret`. */
async function v1(secret: string, message: Uint8Array<ArrayBuffer>): Promise<string> {
  const key = await crypto.subtle.importKey(
    "raw",
    bytesOfBase64(secret.slice("whsec_".length)),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  return `v1,${base64Of(new Uint8Array(await crypto.subtle.sign("HMAC", key, message)))}`;
}

/** A `v2a` signature for `endpoint` with the `whsk_` key whose public key is the `whpk_` `pub`. */
async function v2a(
  signing: string,
  pub: string,
  endpoint: string,
  message: Uint8Array<ArrayBuffer>,
): Promise<string> {
  const key = await crypto.subtle.importKey(
    "jwk",
    {
      kty: "OKP",
      crv: "Ed25519",
      d: base64UrlOf(bytesOfBase64(signing.slice("whsk_".length))),
      x: base64UrlOf(bytesOfBase64(pub.slice("whpk_".length))),
    },
    { name: "Ed25519" },
    false,
    ["sign"],
  );
  const digest = new Uint8Array(
    await crypto.subtle.digest(
      "SHA-256",
      concat(new TextEncoder().encode(`${endpoint}.`), message),
    ),
  );
  return `v2a,${base64Of(new Uint8Array(await crypto.subtle.sign("Ed25519", key, digest)))}`;
}

/** The headers of a delivery of `body` signed by each of `signers` at {@link SIGNED_AT}. */
async function signed(
  body: string,
  ...signers: readonly ((message: Uint8Array<ArrayBuffer>) => Promise<string>)[]
): Promise<Record<string, string>> {
  const message = new TextEncoder().encode(`${EVENT_ID}.${SIGNED_AT}.${body}`);
  const signatures = await Promise.all(signers.map((sign) => sign(message)));
  return {
    "webhook-id": EVENT_ID,
    "webhook-timestamp": String(SIGNED_AT),
    "webhook-signature": signatures.join(" "),
    "content-type": "application/json",
  };
}

/** A full Notification payload of a failed Run, as the engine writes it. */
const FULL = {
  type: "notification.run_failed",
  timestamp: "2026-01-05T08:59:58Z",
  format: "0.1",
  data: {
    event_id: EVENT_ID,
    event: "run_failed",
    run_id: "nightly-prices",
    schedule_id: null,
    product: "collie",
    title: "nightly-prices failed",
    text: "The Run ended on an error.",
    link: "http://127.0.0.1:7777/runs/nightly-prices",
  },
};

/** {@link FULL} as the helper reads it. */
const FULL_READ: NotificationPayload = {
  type: "notification.run_failed",
  timestamp: new Date("2026-01-05T08:59:58Z"),
  format: "0.1",
  data: {
    eventId: EVENT_ID,
    event: "run_failed",
    runId: "nightly-prices",
    scheduleId: null,
    product: "collie",
    title: "nightly-prices failed",
    text: "The Run ended on an error.",
    link: "http://127.0.0.1:7777/runs/nightly-prices",
  },
};

const v1Keys = signedVector("rotation_two_v1_keys").keys;
const [current = "", previous = ""] = v1Keys;
/** A secret of no channel this test signs for. */
const UNKNOWN = "whsec_c2VjcmV0IG9mIGFub3RoZXIgY2hhbm5lbCEhISE=";

/** The error `verify` fails with. */
async function failure(verify: Promise<NotificationPayload>): Promise<unknown> {
  try {
    await verify;
  } catch (error) {
    return error;
  }
  throw new Error("the call verified");
}

/** The reason `error` names, after checking it is `webhook_invalid`. */
function reasonOf(error: unknown): WebhookInvalidReason | undefined {
  if (!(error instanceof WebhookInvalidError)) {
    throw new Error(`no webhook_invalid: ${String(error)}`);
  }
  expect(error.code).toBe("webhook_invalid");
  expect(error.group).toBe("program");
  expect(error.retryable).toBe(false);
  return error.typedDetails?.reason;
}

const EXPECTED: Readonly<Record<VerificationCase["outcome"], WebhookInvalidReason>> = {
  // A vector's body is no Notification payload: one whose signature
  // verifies is refused only once its body is read.
  verified: "malformed",
  no_matching_signature: "signature",
  timestamp_too_old: "timestamp",
  timestamp_too_new: "timestamp",
  timestamp_invalid: "timestamp",
  missing_header: "missing_header",
};

describe("verifyWebhook", { tags: ["story:11-271"] }, () => {
  test.each(vectors.verifications.map((each) => [each.name, each] as const))(
    "refuses the vector %s for its reason, or passes its signature",
    async (_name, vector) => {
      const headers: Record<string, string> = {};
      for (const [name, value] of [
        ["webhook-id", vector.id],
        ["webhook-timestamp", vector.timestamp],
        ["webhook-signature", vector.signature],
      ] as const) {
        if (value !== null) {
          headers[name] = value;
        }
      }
      const options: VerifyWebhookOptions = {
        at: at(vector.now),
        ...(vector.endpoint === null ? {} : { endpoint: vector.endpoint }),
      };

      const error = await failure(verifyWebhook(vector.body, headers, vector.trusted, options));

      expect(reasonOf(error)).toBe(EXPECTED[vector.outcome]);
    },
  );

  test.each([
    ["full", FULL, FULL_READ],
    [
      "thin",
      { ...FULL, data: { event_id: EVENT_ID, run_id: "nightly-prices" } },
      { ...FULL_READ, data: { eventId: EVENT_ID, runId: "nightly-prices" } },
    ],
  ] as const)("reads back a signed %s payload", async (_name, payload, read) => {
    const body = JSON.stringify(payload);
    const headers = await signed(body, (message) => v1(current, message));

    const verified = await verifyWebhook(body, headers, current, { at: at(SIGNED_AT + 1) });

    expect(verified).toEqual(read);
    expect(verified.timestamp).toBeInstanceOf(Date);
  });

  test("reads a body given as bytes and headers given as a `Headers`", async () => {
    const body = JSON.stringify(FULL);
    const headers = new Headers(await signed(body, (message) => v1(current, message)));

    const verified = await verifyWebhook(new TextEncoder().encode(body), headers, current, {
      at: at(SIGNED_AT),
    });

    expect(verified).toEqual(FULL_READ);
  });

  test.each([
    ["the current secret", current],
    ["the previous secret", previous],
  ])("accepts a rotation signed with both secrets by %s", async (_name, trusted) => {
    const body = JSON.stringify(FULL);
    const headers = await signed(
      body,
      (message) => v1(current, message),
      (message) => v1(previous, message),
    );
    const options = { at: at(SIGNED_AT) };

    expect(await verifyWebhook(body, headers, trusted, options)).toEqual(FULL_READ);
    expect(await verifyWebhook(body, headers, [UNKNOWN, trusted], options)).toEqual(FULL_READ);
  });

  test("verifies a `v2a` signature with the public key, for its endpoint only", async () => {
    const signing = signedVector("v2a_published");
    const [secretKey = ""] = signing.keys;
    const [publicKey = ""] = verification("v2a_published").trusted;
    const endpoint = signing.endpoint ?? "";
    const body = JSON.stringify(FULL);
    const headers = await signed(body, (message) => v2a(secretKey, publicKey, endpoint, message));

    const verified = await verifyWebhook(body, headers, publicKey, {
      at: at(SIGNED_AT),
      endpoint,
    });
    const elsewhere = await failure(
      verifyWebhook(body, headers, publicKey, { at: at(SIGNED_AT), endpoint: "ep_another" }),
    );

    expect(verified).toEqual(FULL_READ);
    expect(reasonOf(elsewhere)).toBe("signature");
  });

  test.each([
    [
      "a forged body",
      JSON.stringify({ type: "notification.run_finished" }),
      SIGNED_AT,
      "signature",
    ],
    ["a call replayed six minutes later", undefined, SIGNED_AT + 6 * 60, "timestamp"],
    ["a call dated six minutes ahead", undefined, SIGNED_AT - 6 * 60, "timestamp"],
  ] as const)("refuses %s", async (_name, forged, now, expected) => {
    const body = JSON.stringify(FULL);
    const headers = await signed(body, (message) => v1(current, message));

    const error = await failure(verifyWebhook(forged ?? body, headers, current, { at: at(now) }));

    expect(reasonOf(error)).toBe(expected);
  });

  test("accepts an older call within a wider tolerance", async () => {
    const body = JSON.stringify(FULL);
    const headers = await signed(body, (message) => v1(current, message));

    const verified = await verifyWebhook(body, headers, current, {
      at: at(SIGNED_AT + 6 * 60),
      tolerance: 10 * 60 * 1000,
    });

    expect(verified).toEqual(FULL_READ);
  });

  test("reads a newer minor and keeps an unknown event as received", async () => {
    const body = JSON.stringify({
      type: "notification.crawl_drained",
      timestamp: "2026-01-05T09:00:00Z",
      format: "0.7",
      data: {
        event_id: EVENT_ID,
        event: "crawl_drained",
        run_id: "nightly-prices",
        product: "foxhound",
        title: "nightly-prices drained",
        text: "The crawl drained.",
        severity: "info",
      },
      delivery_attempt: 2,
    });
    const headers = await signed(body, (message) => v1(current, message));

    const verified = await verifyWebhook(body, headers, current, { at: at(SIGNED_AT) });

    expect(verified.type).toBe("notification.crawl_drained");
    expect(verified.format).toBe("0.7");
    expect(verified.data).toMatchObject({ event: "crawl_drained", product: "foxhound" });
  });

  test.each([
    ["another major", JSON.stringify({ ...FULL, format: "1.0" })],
    ["no payload", '{"test": 2432232314}'],
    ["no JSON", "ok"],
  ])("refuses a body of %s as malformed", async (_name, body) => {
    const headers = await signed(body, (message) => v1(current, message));

    const error = await failure(verifyWebhook(body, headers, current, { at: at(SIGNED_AT) }));

    expect(reasonOf(error)).toBe("malformed");
  });

  test("matches header names in any case", async () => {
    const body = JSON.stringify(FULL);
    const headers = Object.fromEntries(
      Object.entries(await signed(body, (message) => v1(current, message))).map(([name, value]) => [
        name.toUpperCase(),
        value,
      ]),
    );

    expect(await verifyWebhook(body, headers, current, { at: at(SIGNED_AT) })).toEqual(FULL_READ);
  });

  test("refuses a call signed long ago by the system's clock", async () => {
    const body = JSON.stringify(FULL);
    const headers = await signed(body, (message) => v1(current, message));

    expect(reasonOf(await failure(verifyWebhook(body, headers, current)))).toBe("timestamp");
  });

  test.each([
    ["no secret", [], undefined],
    ["no prefix", ["MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw"], undefined],
    ["an empty secret", ["whsec_"], undefined],
    ["a signing key", ["whsk_VQMJMCF2TCBiITJzbZB7z+0sHSkvbmqQaNgo84ExILU="], "ep_1"],
    [
      "a public key with no endpoint",
      ["whpk_2zPwzAhd9xyyTX6hydEXQjvUWwvpqA/DmAUXdzvZGyU="],
      undefined,
    ],
    ["an endpoint with a dot", ["whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw"], "ep.1"],
  ] as const)("refuses %s before reading the call", async (_name, secrets, endpoint) => {
    const error = await failure(
      verifyWebhook("", {}, secrets, endpoint === undefined ? {} : { endpoint }),
    );

    if (!(error instanceof ProgramError) || error instanceof WebhookInvalidError) {
      throw new Error(`no invalid_params: ${String(error)}`);
    }
    expect(error.code).toBe("invalid_params");
    expect(error.details).toMatchObject({ operation: "verifyWebhook" });
  });
});
