// A Run's Key-value store (ADR 0103): JSON values by key, kept in the
// Run's run state, so a Run resumed by its ID, here or on another machine,
// finds them. A null from `get` always means an absent key, since `set`'s
// type takes no top-level null (G3); an untyped caller's null fails in the
// engine with `value_invalid`, naming `delete`.

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

/** What a Run's store reaches its engine through. */
export interface KeyValueStoreHost {
  /** The committed value of `key` in the Run's store, or `null` when it is absent. */
  get(runId: string, key: string): Promise<JsonValue>;
  /** Commits `value` for `key`. */
  set(runId: string, key: string, value: StoredValue): Promise<void>;
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

  /** The value stored for `key`, as committed, or `null` when the key is absent. */
  async get(key: string): Promise<JsonValue> {
    this.#ensureOpen("key_value_store.get");
    return this.#host.get(this.#runId, key);
  }

  /** Commits `value` for `key`, replacing the one it held: the last commit wins. */
  async set(key: string, value: StoredValue): Promise<void> {
    this.#ensureOpen("key_value_store.set");
    await this.#host.set(this.#runId, key, value);
  }

  /** Removes `key`, and answers whether it existed. */
  async delete(key: string): Promise<boolean> {
    this.#ensureOpen("key_value_store.delete");
    return this.#host.delete(this.#runId, key);
  }
}
