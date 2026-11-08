// The Secret canary search (ADR 0067), as Rust's `Canaries::search` does it:
// every place an output holds a canary, raw and in Goodfetch's encodings
// (JSON-escaped, URL-encoded, base64 standard or URL-safe at any alignment,
// hex in either case), with the output and the byte offset of each hit.
// A test searches what the SDK shows, its `toString`, errors and logs, for
// canaries it planted through `goodfetch-test-env` or the Proving ground.
// Hits name a canary by ID, never by value.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { TestSupportError } from "./errors.ts";
import type { SecretKind } from "./generated/secret-kinds.ts";

export { SECRET_KINDS, type SecretKind } from "./generated/secret-kinds.ts";

/** The encodings a value is searched in. */
export type Encoding = "raw" | "json_escaped" | "url_encoded" | "base64" | "hex";

/** Whether a hit is where the canary was planted, or a leak. */
export type Placement = "planted" | "leaked";

/** A planted canary. */
export interface Canary {
  /** Its ID, which hits name it by. */
  id: string;
  kind: SecretKind;
  value: string;
  /** The outputs the test planted it in, whose hits are planted copies. */
  plantedIn?: readonly string[] | undefined;
}

/** An output a search reads: bytes or text the test holds, or a file or folder. */
export type CanaryOutput =
  | { name: string; bytes: Uint8Array }
  | { name: string; text: string }
  | { path: string };

/** One place an output holds a canary. */
export interface CanaryHit {
  canary: string;
  kind: SecretKind;
  encoding: Encoding;
  /** The output's name, or the file's path. */
  output: string;
  /** The byte offset where the encoded value starts. */
  offset: number;
  placement: Placement;
}

/** The marks RFC 3986 leaves unreserved. */
const UNRESERVED = /^[A-Za-z0-9\-._~]$/;
/** The marks a browser keeps as it encodes a form, which writes a space `+`. */
const FORM_KEPT = /^[A-Za-z0-9*\-._]$/;
/** The marks JavaScript's `encodeURIComponent` keeps. */
const URI_COMPONENT_KEPT = /^[A-Za-z0-9\-_.!~*'()]$/;

/** `bytes` percent-encoded, every byte but those `kept` matches. */
function urlEncoded(bytes: Uint8Array, kept: RegExp): string {
  let encoded = "";
  for (const byte of bytes) {
    const character = String.fromCharCode(byte);
    encoded +=
      byte < 0x80 && kept.test(character)
        ? character
        : `%${byte.toString(16).toUpperCase().padStart(2, "0")}`;
  }
  return encoded;
}

/** The base64 characters that encode `value` alone, starting `alignment` bytes into a group. */
function base64At(value: Uint8Array, alignment: number, alphabet: "base64" | "base64url"): string {
  const padded = Buffer.concat([Buffer.alloc(alignment), value]);
  const encoded = padded.toString(alphabet).replace(/=+$/, "");
  const start = [0, 2, 3][alignment] ?? 0;
  const end = Math.floor(padded.length / 3) * 4 + (padded.length % 3);
  return start >= end ? "" : encoded.slice(start, end);
}

/** Each searched form of a canary's value, a form an earlier encoding gives left out. */
function forms(canary: Canary): [Encoding, Buffer][] {
  const value = Buffer.from(canary.value, "utf8");
  const found: [Encoding, Buffer][] = [];
  const add = (encoding: Encoding, form: string | Buffer): void => {
    const bytes = typeof form === "string" ? Buffer.from(form, "utf8") : form;
    if (bytes.length > 0 && found.every(([, earlier]) => !earlier.equals(bytes))) {
      found.push([encoding, bytes]);
    }
  };
  add("raw", value);
  add("json_escaped", JSON.stringify(canary.value).slice(1, -1));
  add("url_encoded", urlEncoded(value, UNRESERVED));
  add("url_encoded", urlEncoded(value, FORM_KEPT).replaceAll("%20", "+"));
  add("url_encoded", urlEncoded(value, URI_COMPONENT_KEPT));
  for (const alphabet of ["base64", "base64url"] as const) {
    for (const alignment of [0, 1, 2]) add("base64", base64At(value, alignment, alphabet));
  }
  add("hex", value.toString("hex"));
  add("hex", value.toString("hex").toUpperCase());
  return found;
}

/** `path` itself when it is a file, else every file under it, in path order. */
function files(path: string): string[] {
  if (!statSync(path).isDirectory()) return [path];
  return readdirSync(path)
    .toSorted()
    .flatMap((entry) => files(join(path, entry)));
}

function outputBytes(output: CanaryOutput): [string, Buffer][] {
  if ("path" in output) {
    try {
      return files(output.path).map((file) => [file, readFileSync(file)]);
    } catch (error) {
      throw new TestSupportError(
        "canary_output_unreadable",
        `reading ${output.path} for Secret canaries`,
        { cause: error },
      );
    }
  }
  return [
    [output.name, "bytes" in output ? Buffer.from(output.bytes) : Buffer.from(output.text, "utf8")],
  ];
}

/**
 * Every place `outputs` hold one of `canaries`: in output order, then by
 * offset, then in canary and encoding order.
 *
 * @throws {TestSupportError} `canary_output_unreadable` when a path cannot be read.
 */
export function searchCanaries(
  canaries: readonly Canary[],
  outputs: readonly CanaryOutput[],
): CanaryHit[] {
  const searched = canaries.map((canary) => ({ canary, forms: forms(canary) }));
  const hits: CanaryHit[] = [];
  for (const output of outputs) {
    for (const [name, bytes] of outputBytes(output)) {
      const found: { hit: CanaryHit; order: number; form: number }[] = [];
      searched.forEach(({ canary, forms: canaryForms }, order) => {
        const placement: Placement = canary.plantedIn?.includes(name) ? "planted" : "leaked";
        canaryForms.forEach(([encoding, form], formIndex) => {
          for (let at = bytes.indexOf(form); at !== -1; at = bytes.indexOf(form, at + 1)) {
            found.push({
              hit: {
                canary: canary.id,
                kind: canary.kind,
                encoding,
                output: name,
                offset: at,
                placement,
              },
              order,
              form: formIndex,
            });
          }
        });
      });
      found.sort((a, b) => a.hit.offset - b.hit.offset || a.order - b.order || a.form - b.form);
      for (const { hit } of found) {
        const last = hits.at(-1);
        const same =
          last !== undefined &&
          last.output === hit.output &&
          last.offset === hit.offset &&
          last.canary === hit.canary &&
          last.encoding === hit.encoding;
        if (!same) hits.push(hit);
      }
    }
  }
  return hits;
}
