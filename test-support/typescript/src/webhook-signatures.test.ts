// The shared Standard Webhooks vectors hold in TypeScript too: Node's own
// HMAC-SHA256 and ed25519 make every signed vector's signatures and check
// every verification case's, so the SDKs' helpers can share the file.
import {
  createHash,
  createHmac,
  createPrivateKey,
  createPublicKey,
  type KeyObject,
  sign,
  timingSafeEqual,
  verify,
} from "node:crypto";
import { describe, expect, test } from "vitest";
import { webhookSignatures } from "./webhook-signatures.ts";

// The DER prefixes of an ed25519 key's PKCS #8 and SPKI forms (RFC 8410),
// before its 32 raw bytes.
const PRIVATE_PREFIX = Buffer.from("302e020100300506032b657004220420", "hex");
const PUBLIC_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

interface Message {
  id: string;
  timestamp: string;
  body: string;
  endpoint: string | null;
}

function signed(message: Message): string {
  return `${message.id}.${message.timestamp}.${message.body}`;
}

/** The SHA-256 a `v2a` signature covers: the endpoint ID, then the signed text. */
function digest(message: Message): Buffer {
  return createHash("sha256")
    .update(`${message.endpoint}.${signed(message)}`)
    .digest();
}

function raw(key: string, prefix: string): Buffer {
  return Buffer.from(key.slice(prefix.length), "base64");
}

function privateKey(key: string): KeyObject {
  return createPrivateKey({
    key: Buffer.concat([PRIVATE_PREFIX, raw(key, "whsk_")]),
    format: "der",
    type: "pkcs8",
  });
}

function publicKey(key: string): KeyObject {
  return createPublicKey({
    key: Buffer.concat([PUBLIC_PREFIX, raw(key, "whpk_")]),
    format: "der",
    type: "spki",
  });
}

function hmac(key: string, message: Message): Buffer {
  return createHmac("sha256", raw(key, "whsec_")).update(signed(message)).digest();
}

/** One key's `webhook-signature` entry. */
function signature(key: string, message: Message): string {
  return key.startsWith("whsk_")
    ? `v2a,${sign(null, digest(message), privateKey(key)).toString("base64")}`
    : `v1,${hmac(key, message).toString("base64")}`;
}

/** Whether a trusted key verifies one entry of a `webhook-signature` of the same version. */
function verifies(key: string, entry: string, message: Message): boolean {
  const [version, encoded] = entry.split(",", 2);
  if (encoded === undefined) return false;
  const bytes = Buffer.from(encoded, "base64");
  if (key.startsWith("whpk_")) {
    return (
      version === "v2a" &&
      bytes.length === 64 &&
      verify(null, digest(message), publicKey(key), bytes)
    );
  }
  const expected = hmac(key, message);
  return version === "v1" && bytes.length === expected.length && timingSafeEqual(bytes, expected);
}

describe("webhookSignatures", () => {
  test("hold the specification's published v1 and v2a vectors", () => {
    const { signed: vectors } = webhookSignatures();

    expect(vectors.filter((vector) => vector.source !== null).map((vector) => vector.name)).toEqual(
      ["v1_published", "v2a_published"],
    );
  });

  test("give each signed vector's signatures under Node's HMAC-SHA256 and ed25519", () => {
    const { signed: vectors } = webhookSignatures();

    const misses = vectors.flatMap((vector) => {
      const message = { ...vector, timestamp: String(vector.timestamp) };
      const sent = vector.keys.map((key) => signature(key, message)).join(" ");
      return sent === vector.signature ? [] : [`${vector.name}: ${sent}`];
    });

    expect(misses).toEqual([]);
  });

  test("verify a case's signature under a trusted key exactly when it verifies", () => {
    const { verifications } = webhookSignatures();

    const misses = verifications
      .filter((check) => check.outcome === "verified" || check.outcome === "no_matching_signature")
      .flatMap((check) => {
        const message = {
          id: check.id ?? "",
          timestamp: check.timestamp ?? "",
          body: check.body,
          endpoint: check.endpoint,
        };
        const entries = (check.signature ?? "").split(" ");
        const verified = check.trusted.some((key) =>
          entries.some((entry) => verifies(key, entry, message)),
        );
        return verified === (check.outcome === "verified") ? [] : [check.name];
      });

    expect(misses).toEqual([]);
  });
});
