// A Run's Key-value store (ADR 0103): JSON values, and bytes with their
// content type, by key, kept in the Run's run state, so a Run resumed by its
// ID, here or on another machine, finds them. A null from `get` always means
// an absent key, since `set`'s type takes no top-level null (G3); an untyped
// caller's null fails in the engine with `value_invalid`, naming `delete`.
// One method never answers two types (S2): `get` on bytes fails with
// `value_invalid` naming `getBytes`, and `getBytes` on JSON naming `get`.

/** A JSON value. */
export type JsonValue =
  | string
  | number
  | boolean
  | null
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue };

/** What the store keeps: any JSON value but a top-level null. A nested null is kept. */
export type StoredValue = Exclude<JsonValue, null>;

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
  get(runId: string, key: string): Promise<JsonValue>;
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
   */
  async get(key: string): Promise<JsonValue> {
    this.#ensureOpen("key_value_store.get");
    return this.#host.get(this.#runId, key);
  }

  /** Commits `value` for `key`, replacing the one it held: the last commit wins. */
  async set(key: string, value: StoredValue): Promise<void> {
    this.#ensureOpen("key_value_store.set");
    await this.#host.set(this.#runId, key, value);
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
