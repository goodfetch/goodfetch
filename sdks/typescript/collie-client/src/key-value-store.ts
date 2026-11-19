// A Run's Key-value store (ADR 0103): JSON values, and bytes with their
// content type, by key, kept in the Run's run state, so a Run resumed by its
// ID, here or on another machine, finds them. A null from `get` always means
// an absent key, since `set`'s type takes no top-level null (G3); an untyped
// caller's null fails in the engine with `value_invalid`, naming `delete`.
// One method never answers two types (S2): `get` on bytes fails with
// `value_invalid` naming `getBytes`, and `getBytes` on JSON naming `get`.
// A value may be typed by a schema in the program's own types (ADR 0082):
// `set` sends the schema's input, which its parse reads back, and `get`
// answers the output its parse builds. Only JSON crosses: `set` refuses a
// value that is not JSON before sending anything, naming its path.

import { ValueInvalidError } from "./generated/errors.ts";
import { nonJSON, type JSONValue } from "./json.ts";
import { parse, type Schema, type SchemaInput, type SchemaOutput } from "./schema.ts";

/** What the store keeps: any JSON value but a top-level null. A nested null is kept. */
export type StoredValue = Exclude<JSONValue, null>;

/** Bytes the store keeps, with their content type. */
export interface StoredBytes {
  /** The bytes. */
  readonly content: Uint8Array;
  /** Their content type, such as `image/png`. */
  readonly contentType: string;
}

/** What a Run's store reaches its engine through. */
export interface KeyValueStoreHost {
  /** The committed value of `key` in the Run's store, or `null` when it is absent. */
  get(runId: string, key: string): Promise<JSONValue>;
  /** Commits `value` for `key`. */
  set(runId: string, key: string, value: StoredValue): Promise<void>;
  /** The committed bytes of `key` in the Run's store, or `null` when it is absent. */
  getBytes(runId: string, key: string): Promise<StoredBytes | null>;
  /** Commits `content` of `contentType` for `key`. */
  setBytes(runId: string, key: string, content: Uint8Array, contentType: string): Promise<void>;
  /** Removes `key`, answering whether it existed. */
  delete(runId: string, key: string): Promise<boolean>;
}

/**
 * A Run's Key-value store: `run.keyValueStore`. A key is 1 to 512 Unicode
 * scalar values with no control character; another fails with
 * `value_invalid`, naming it.
 */
export class KeyValueStore {
  readonly #host: KeyValueStoreHost;
  readonly #runId: string;
  readonly #ensureOpen: (operation: string) => void;

  /** @internal A Run opens its store; a program never constructs one. */
  constructor(host: KeyValueStoreHost, runId: string, ensureOpen: (operation: string) => void) {
    this.#host = host;
    this.#runId = runId;
    this.#ensureOpen = ensureOpen;
  }

  /**
   * The JSON value stored for `key`, as committed, or `null` when the key is
   * absent. A key holding bytes fails with `value_invalid`, naming `getBytes`.
   * Given a schema, it answers the output the schema's own parse builds, and
   * a stored value the schema rejects fails with `value_invalid`, naming the
   * first issue's path.
   */
  async get(key: string): Promise<JSONValue>;
  async get<S extends Schema>(key: string, schema: S): Promise<SchemaOutput<S> | null>;
  async get(key: string, schema?: Schema): Promise<unknown> {
    this.#ensureOpen("key_value_store.get");
    const stored = await this.#host.get(this.#runId, key);
    if (schema === undefined || stored === null) {
      return stored;
    }
    const parsed = await parse(schema, stored);
    if ("issues" in parsed) {
      throw schemaRefusal(key, "stored", parsed.issues);
    }
    return parsed.value;
  }

  /**
   * Commits `value` for `key`, replacing the one it held: the last commit
   * wins. Given a schema, `value` has its input type, is parsed by it before
   * anything is sent and is stored as given, the form the schema reads back.
   * A value that is not JSON (`NaN`, a `Map`, a `bigint`) or that the schema
   * rejects fails with `value_invalid`, naming its path, and nothing is sent.
   */
  async set(key: string, value: StoredValue): Promise<void>;
  async set<S extends Schema>(
    key: string,
    value: Exclude<SchemaInput<S>, null>,
    schema: S,
  ): Promise<void>;
  async set(key: string, value: unknown, schema?: Schema): Promise<void> {
    this.#ensureOpen("key_value_store.set");
    const refusal = nonJSON(value);
    if (refusal) {
      throw new ValueInvalidError(
        `The Key-value store refused the value of ${JSON.stringify(key)}: ${refusal.found} at ${refusal.path || "its root"} is not JSON.`,
        {
          fix: "Encode it as JSON first: a string, a finite number, a boolean, null, an array or a plain object (a `Date` as an ISO string, a `Map` as an object, a `bigint` as a string).",
          details: { key, refused: "value", reason: "value_not_json", path: refusal.path },
        },
      );
    }
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `nonJSON` found nothing that is not JSON.
    const json = value as StoredValue;
    if (schema !== undefined) {
      const parsed = await parse(schema, json);
      if ("issues" in parsed) {
        throw schemaRefusal(key, "given", parsed.issues);
      }
    }
    await this.#host.set(this.#runId, key, json);
  }

  /**
   * The bytes stored for `key` with their content type, as committed, or `null`
   * when the key is absent. A key holding JSON fails with `value_invalid`,
   * naming `get`.
   */
  async getBytes(key: string): Promise<StoredBytes | null> {
    this.#ensureOpen("key_value_store.get_bytes");
    return this.#host.getBytes(this.#runId, key);
  }

  /**
   * Commits `content`, of `contentType`, for `key`, replacing what it held: the
   * last commit wins. Bytes over 1 MiB are kept as a file of the Run, which its
   * Checkpoints carry.
   */
  async setBytes(key: string, content: Uint8Array, contentType: string): Promise<void> {
    this.#ensureOpen("key_value_store.set_bytes");
    await this.#host.setBytes(this.#runId, key, content, contentType);
  }

  /** Removes `key`, and answers whether it existed. */
  async delete(key: string): Promise<boolean> {
    this.#ensureOpen("key_value_store.delete");
    return this.#host.delete(this.#runId, key);
  }
}

/** `value_invalid` for the value of `key` the schema rejected, `given` to `set` or `stored`. */
function schemaRefusal(
  key: string,
  side: "given" | "stored",
  issues: readonly { readonly path: string; readonly message: string }[],
): ValueInvalidError {
  const listed = issues.map((issue) => `${issue.path || "its root"}: ${issue.message}`).join("; ");
  return new ValueInvalidError(
    `The schema rejected the value ${side} for ${JSON.stringify(key)}: ${listed}.`,
    {
      fix:
        side === "given"
          ? "Give a value the schema accepts."
          : "Read it with the schema it was stored under, or without one, and store it again in the new shape.",
      details: { key, refused: "value", reason: "value_schema", path: issues[0]?.path ?? "" },
    },
  );
}
