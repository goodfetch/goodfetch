// The verification helper of a `webhook` channel's delivery (ADRs 0017,
// 0027 and 0030; issue 1391): pure, with no engine, no Connection and no
// Node built-in, so a receiver calls it on whatever server or edge runtime
// takes the call. It checks the Standard Webhooks headers, `webhook-id`,
// `webhook-timestamp` and `webhook-signature`, against the secrets given,
// the current one and the previous one while a secret rotates, then reads
// the body as the `NotificationPayload` the engine sent. A call it refuses
// fails with `webhook_invalid`, whose details name the reason.
//
// A `whsec_` secret verifies a `v1` signature, HMAC-SHA256 over
// `<id>.<timestamp>.<body>`; a `whpk_` public key verifies a `v2a` one,
// Ed25519 over the SHA-256 of `<endpoint>.<id>.<timestamp>.<body>`, for the
// endpoint ID `options.endpoint` names. WebCrypto compares each signature in
// constant time, and an entry of a version no secret has, or that is no
// Base64, matches nothing, as the engine's reference verifier does.
import { isRecord } from "./framing.ts";
import { ProgramError, WebhookInvalidError } from "./generated/errors.ts";
import type { NotificationPayload, WebhookInvalidReason } from "./generated/types.ts";
import { collieWire } from "./generated/wire.ts";
import { runtimeMessage } from "./goodfetch-error.ts";
import { fromWire } from "./wire.ts";

/** How far a delivery's timestamp may be from now, either way, by default. */
const TOLERANCE_MS = 5 * 60 * 1000;

/** The major of the Notification payload this SDK reads, every minor of it. */
const FORMAT_MAJOR = 0;

/** The headers a delivery carries, as Standard Webhooks names them. */
const ID = "webhook-id";
const TIMESTAMP = "webhook-timestamp";
const SIGNATURE = "webhook-signature";

/** The name `invalid_params` names for a secret the helper cannot use. */
const OPERATION = "verifyWebhook";

const BASE64 = /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/;
const FORMAT = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/;
const SECONDS = /^[0-9]+$/;

/** A delivery's body, as received, before any parsing. */
export type WebhookBody = string | Uint8Array | ArrayBuffer;

/** A delivery's headers: a `Headers`, or a record such as Node's, names in any case. */
export type WebhookHeaders =
  | Headers
  | Readonly<Record<string, string | readonly string[] | undefined>>;

/** What `verifyWebhook` takes beside the call. */
export interface VerifyWebhookOptions {
  /** How far the delivery's timestamp may be from now, either way, in milliseconds: 5 minutes by default. */
  readonly tolerance?: number | undefined;
  /** The endpoint ID the channel's `whsk_` key signs for, which a `whpk_` secret needs; never sent with the call. */
  readonly endpoint?: string | undefined;
  /** The time to verify at in place of the system's, such as a test's. */
  readonly at?: Date | undefined;
}

/**
 * Verifies that a call is a `webhook` channel's delivery signed with one of
 * `secret`'s secrets less than 5 minutes ago, and reads its body: `body` as
 * received, before any parsing, and `headers` as received, whose names are
 * matched in any case. `secret` is the channel's `webhook.secret`, or a list
 * of the current and previous ones while it rotates.
 *
 * Fails with `webhook_invalid` for a call that is no such delivery, its
 * details naming why, and with `invalid_params` for no secret, a secret that
 * is neither a `whsec_` secret nor a `whpk_` public key, or a `whpk_` key
 * with no endpoint.
 */
export async function verifyWebhook(
  body: WebhookBody,
  headers: WebhookHeaders,
  secret: string | readonly string[],
  options: VerifyWebhookOptions = {},
): Promise<NotificationPayload> {
  const trusted = await trustedOf(typeof secret === "string" ? [secret] : secret, options.endpoint);
  const call = callOf(headers);
  if (!SECONDS.test(call.timestamp) || !Number.isSafeInteger(Number(call.timestamp))) {
    throw invalid("timestamp", "`webhook-timestamp` is no Unix time in seconds");
  }
  const now = Math.floor((options.at ?? new Date()).getTime() / 1000);
  const tolerance = options.tolerance ?? TOLERANCE_MS;
  if (Math.abs(now - Number(call.timestamp)) * 1000 > tolerance) {
    throw invalid(
      "timestamp",
      `the call was signed more than ${tolerance} milliseconds from now: a delayed or replayed call`,
    );
  }
  const bytes = bytesOf(body);
  if (!(await trusted.signed(`${call.id}.${call.timestamp}.`, bytes, call.signature))) {
    throw invalid("signature", "no signature of `webhook-signature` matches a secret given");
  }
  return payloadOf(bytes);
}

/** The three Standard Webhooks headers of a call. */
interface Call {
  readonly id: string;
  readonly timestamp: string;
  readonly signature: string;
}

/** The first value of each header, whatever its name's case. */
function callOf(headers: WebhookHeaders): Call {
  const read = (name: string): string => {
    const value = headerOf(headers, name);
    if (value === undefined) {
      throw invalid("missing_header", `the call has no \`${name}\` header`);
    }
    return value;
  };
  return { id: read(ID), timestamp: read(TIMESTAMP), signature: read(SIGNATURE) };
}

function headerOf(headers: WebhookHeaders, name: string): string | undefined {
  if (headers instanceof Headers) {
    return headers.get(name) ?? undefined;
  }
  for (const [key, value] of Object.entries(headers)) {
    if (key.toLowerCase() !== name || value === undefined) {
      continue;
    }
    const first: string | undefined = typeof value === "string" ? value : value[0];
    if (first !== undefined) {
      return first;
    }
  }
  return undefined;
}

/** A secret, read: a `whsec_` secret verifies a `v1` signature, a `whpk_` public key a `v2a` one. */
interface Key {
  readonly version: "v1" | "v2a";
  readonly key: CryptoKey;
}

/** The secrets a call may be signed with, and the endpoint their `v2a` signatures name. */
interface Trusted {
  /** Whether an entry of `signatures` is a key's signature of `signed`, the ID and timestamp, followed by `body`. */
  signed(signed: string, body: Uint8Array, signatures: string): Promise<boolean>;
}

/** Reads `secrets`, refusing one that is no secret and a `whpk_` key without `endpoint`. */
async function trustedOf(
  secrets: readonly string[],
  endpoint: string | undefined,
): Promise<Trusted> {
  if (secrets.length === 0) {
    throw refused(
      "no secret was given",
      "Pass the channel's `webhook.secret`, and the previous one too while it rotates.",
    );
  }
  const keys = await Promise.all(secrets.map(keyOf));
  if (endpoint !== undefined && (endpoint === "" || endpoint.includes("."))) {
    throw refused(
      "the endpoint ID is empty or holds a `.`",
      "Pass the endpoint ID the channel's `whsk_` key signs for.",
    );
  }
  if (endpoint === undefined && keys.some((key) => key.version === "v2a")) {
    throw refused(
      "a `whpk_` key verifies a `v2a` signature, which names an endpoint, and none was given",
      "Pass the endpoint ID the channel's `whsk_` key signs for.",
    );
  }
  const encoder = new TextEncoder();
  return {
    async signed(signed, body, signatures) {
      const message = concat(encoder.encode(signed), body);
      const v2a =
        endpoint === undefined
          ? undefined
          : await sha256(concat(encoder.encode(`${endpoint}.`), message));
      for (const entry of signatures.split(" ")) {
        const comma = entry.indexOf(",");
        const signature = comma === -1 ? undefined : base64(entry.slice(comma + 1));
        if (signature === undefined) {
          continue;
        }
        const version = entry.slice(0, comma);
        for (const key of keys) {
          if (key.version !== version) {
            continue;
          }
          const verified =
            key.version === "v1"
              ? await crypto.subtle.verify("HMAC", key.key, signature, message)
              : v2a !== undefined &&
                signature.length === 64 &&
                (await crypto.subtle.verify("Ed25519", key.key, signature, v2a));
          if (verified) {
            return true;
          }
        }
      }
      return false;
    },
  };
}

/** The refusal of a secret that is neither a `whsec_` secret nor a `whpk_` public key. */
function unreadable(): ProgramError {
  return refused(
    "a secret is neither a `whsec_` secret nor a `whpk_` public key in Base64",
    "Pass the channel's `webhook.secret` as the engine has it, or the `whpk_` public key of its `whsk_` key.",
  );
}

/** Reads a secret by its prefix. */
async function keyOf(secret: string): Promise<Key> {
  if (secret.startsWith("whsec_")) {
    // An empty secret is refused, as WebCrypto refuses an empty HMAC key.
    const bytes = base64(secret.slice("whsec_".length));
    if (bytes === undefined || bytes.length === 0) {
      throw unreadable();
    }
    const key = await crypto.subtle.importKey(
      "raw",
      bytes,
      { name: "HMAC", hash: "SHA-256" },
      false,
      ["verify"],
    );
    return { version: "v1", key };
  }
  const bytes = secret.startsWith("whpk_") ? base64(secret.slice("whpk_".length)) : undefined;
  if (bytes === undefined || bytes.length !== 32) {
    throw unreadable();
  }
  try {
    const key = await crypto.subtle.importKey("raw", bytes, { name: "Ed25519" }, false, ["verify"]);
    return { version: "v2a", key };
  } catch {
    throw unreadable();
  }
}

/** Reads the body as a Notification payload of the major this SDK reads. */
function payloadOf(body: Uint8Array): NotificationPayload {
  let value: unknown;
  try {
    value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(body));
  } catch {
    throw malformed("the body is no JSON");
  }
  if (
    !isRecord(value) ||
    typeof value["type"] !== "string" ||
    typeof value["timestamp"] !== "string" ||
    typeof value["format"] !== "string" ||
    !isRecord(value["data"]) ||
    typeof value["data"]["event_id"] !== "string"
  ) {
    throw malformed("the body is no Notification payload");
  }
  const format = FORMAT.exec(value["format"]);
  if (format === null) {
    throw malformed("the body is no Notification payload");
  }
  if (Number(format[1]) !== FORMAT_MAJOR) {
    throw malformed(`the body's \`format\` is ${value["format"]}, a major this SDK does not read`);
  }
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the signed body is the engine's, checked above for the fields every minor of this major keeps; the rest is read as the catalog types it, as engine payloads are (STYLE.md).
  const payload = fromWire(
    value,
    { type: "NotificationPayload" },
    collieWire.types,
  ) as NotificationPayload;
  if (!(payload.timestamp instanceof Date)) {
    throw malformed("the body is no Notification payload");
  }
  return payload;
}

/** `body`'s bytes, a text one in UTF-8. */
function bytesOf(body: WebhookBody): Uint8Array {
  if (typeof body === "string") {
    return new TextEncoder().encode(body);
  }
  return body instanceof Uint8Array ? body : new Uint8Array(body);
}

/** The bytes of `text`, strict Base64 in its canonical form, `undefined` otherwise. */
function base64(text: string): Uint8Array<ArrayBuffer> | undefined {
  if (!BASE64.test(text)) {
    return undefined;
  }
  const binary = atob(text);
  // A non-canonical last character, whose unused bits are not zero, reads
  // as another text; the engine's Base64 refuses it too.
  if (btoa(binary) !== text) {
    return undefined;
  }
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}

function concat(head: Uint8Array, tail: Uint8Array): Uint8Array<ArrayBuffer> {
  const joined = new Uint8Array(head.length + tail.length);
  joined.set(head);
  joined.set(tail, head.length);
  return joined;
}

async function sha256(bytes: Uint8Array<ArrayBuffer>): Promise<Uint8Array<ArrayBuffer>> {
  return new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
}

/** A call refused before it is checked, as a raw call's params are. */
function refused(reason: string, fix: string): ProgramError {
  return new ProgramError(`${OPERATION}: ${reason}`, {
    code: "invalid_params",
    group: "program",
    retryable: false,
    fix,
    details: { operation: OPERATION },
  });
}

const FIXES: Readonly<Record<WebhookInvalidReason, string>> = {
  signature:
    "Answer the call with an error; when every delivery fails, pass the channel's `webhook.secret`, and the previous one too while it rotates.",
  timestamp:
    "Answer the call with an error; when every delivery fails, check the receiver's clock.",
  malformed:
    "Answer the call with an error; when every delivery fails, update the SDK to one that reads the engine's payload.",
  missing_header:
    "Pass the request's headers as received; a call without them is no `webhook` delivery.",
};

function malformed(whatFailed: string): WebhookInvalidError {
  return invalid("malformed", whatFailed);
}

/** `webhook_invalid` for `reason`, saying what failed. */
function invalid(reason: WebhookInvalidReason, whatFailed: string): WebhookInvalidError {
  const fix = FIXES[reason];
  return new WebhookInvalidError(
    runtimeMessage(`${whatFailed}, so it is no verified \`webhook\` delivery`, { fix }),
    { fix, details: { reason } },
  );
}
