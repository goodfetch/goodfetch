// The Console's offline cache (ADR 0031; spec #592's `offline`): what a
// page keeps in the browser's IndexedDB so it goes on showing run
// summaries while it has no connection to the Daemon. The browser keeps one
// cache per origin, and each major's Console has its own port, so each
// major has its own.
//
// Everything is encrypted with AES-GCM under a cache key the page holds
// only in memory, as a non-extractable key. IndexedDB keeps the key wrapped
// by the Daemon, which wraps it under a key it holds for the sign-in and
// unwraps it only for that sign-in's cookie: once the sign-in expires or is
// revoked, nothing unwraps it, and the cache can no longer be read. A cache
// whose key no longer unwraps is emptied before anything is written again.
// A row is named by an HMAC of its kind and identifier under the same key,
// and a Dataset by an HMAC of its name, so the stored records show no run's
// ID either; only the row's kind is in the clear.
//
// What a screen keeps here is its own call: the Console keeps no secret
// field (`summaries.ts`). Sign-out wipes the cache, and so does a page that
// finds `console.offline_cache` off or itself signed out.

/** The IndexedDB database the cache lives in, one per origin and so per major. */
export const OFFLINE_DATABASE = "goodfetch-console-offline";

/** What a row of the cache holds: a run's Run index row, a run's page, or a Dataset view a person kept offline (Foxhound's). */
export type OfflineKind = "run_summary" | "run_page" | "dataset_view";

/** A new cache key, and its wrapped form, which only its sign-in's cookie unwraps. */
export interface NewOfflineKey {
  readonly key: Uint8Array;
  readonly wrapped: string;
}

/** Where the cache's key comes from: the Daemon, for the page's sign-in. */
export interface OfflineKeys {
  /** A new key; none when the Daemon gives none. */
  create(): Promise<NewOfflineKey | undefined>;
  /** The key `wrapped` holds; none when the sign-in can no longer unwrap it. */
  unwrap(wrapped: string): Promise<Uint8Array | undefined>;
}

/** The cache of one sign-in, open. Its calls never reject: a cache that fails keeps nothing, or reads nothing. */
export interface OfflineCache {
  /** Keeps `value` as the row `id` of `kind`, of the Dataset `dataset` when given, replacing any. */
  put(kind: OfflineKind, id: string, value: unknown, dataset?: string): Promise<void>;
  /** The row `id` of `kind`, if the cache keeps one. */
  get(kind: OfflineKind, id: string): Promise<unknown>;
  /** Every row of `kind` the cache keeps. */
  all(kind: OfflineKind): Promise<unknown[]>;
  /** Deletes every row of the Dataset `dataset`, as its erasure asks. */
  forgetDataset(dataset: string): Promise<void>;
}

const VERSION = 1;
const KEY_STORE = "key";
const ROWS = "rows";
const KEY_RECORD = "key";
const BY_KIND = "kind";
const BY_DATASET = "dataset";
const IV_BYTES = 12;

/** A row as IndexedDB stores it. */
interface StoredRow {
  readonly name: string;
  readonly kind: OfflineKind;
  readonly dataset: string;
  readonly iv: Uint8Array<ArrayBuffer>;
  readonly data: ArrayBuffer;
}

/** The key record as IndexedDB stores it. */
interface StoredKey {
  readonly id: typeof KEY_RECORD;
  readonly wrapped: string;
}

/** What `request` gives once it succeeds. */
async function done<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.addEventListener("success", () => resolve(request.result));
    request.addEventListener("error", () =>
      reject(request.error ?? new Error("an IndexedDB request failed")),
    );
  });
}

/** Resolves once `transaction` commits. */
async function committed(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.addEventListener("complete", () => resolve());
    transaction.addEventListener("error", () =>
      reject(transaction.error ?? new Error("an IndexedDB transaction failed")),
    );
    transaction.addEventListener("abort", () =>
      reject(transaction.error ?? new Error("an IndexedDB transaction aborted")),
    );
  });
}

/** The cache's database, created at its first open. */
async function opened(indexedDB: IDBFactory): Promise<IDBDatabase> {
  const opening = indexedDB.open(OFFLINE_DATABASE, VERSION);
  opening.addEventListener("upgradeneeded", () => {
    const database = opening.result;
    database.createObjectStore(KEY_STORE, { keyPath: "id" });
    const rows = database.createObjectStore(ROWS, { keyPath: "name" });
    rows.createIndex(BY_KIND, "kind");
    rows.createIndex(BY_DATASET, "dataset");
  });
  const database = await done(opening);
  // A wipe, here or in another tab, deletes the database: let it.
  database.addEventListener("versionchange", () => database.close());
  return database;
}

/** Lowercase hex of `bytes`. */
function hex(bytes: ArrayBuffer): string {
  return Array.from(new Uint8Array(bytes), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

/** The cache's keys, from the raw cache key: AES-GCM to seal rows, HMAC to name them; neither extractable. */
async function keysOf(raw: Uint8Array): Promise<{ seal: CryptoKey; name: CryptoKey }> {
  const bytes = new Uint8Array(raw);
  const [seal, name] = await Promise.all([
    crypto.subtle.importKey("raw", bytes, "AES-GCM", false, ["encrypt", "decrypt"]),
    crypto.subtle.importKey("raw", bytes, { name: "HMAC", hash: "SHA-256" }, false, ["sign"]),
  ]);
  bytes.fill(0);
  return { seal, name };
}

/** What `work` gives, or `otherwise` when it fails. */
async function quietly<T>(work: () => Promise<T>, otherwise: T): Promise<T> {
  try {
    return await work();
  } catch {
    return otherwise;
  }
}

/** The cache over `database` under `keys`. */
function cacheOf(database: IDBDatabase, keys: { seal: CryptoKey; name: CryptoKey }): OfflineCache {
  const encoder = new TextEncoder();
  const nameOf = async (...parts: string[]): Promise<string> =>
    hex(await crypto.subtle.sign("HMAC", keys.name, encoder.encode(parts.join("\0"))));
  const openedRow = async (row: StoredRow): Promise<unknown> => {
    const plain = await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: row.iv, additionalData: encoder.encode(row.name) },
      keys.seal,
      row.data,
    );
    return JSON.parse(new TextDecoder().decode(plain));
  };
  return {
    put: async (kind, id, value, dataset) =>
      quietly(async () => {
        const name = await nameOf("row", kind, id);
        const iv = crypto.getRandomValues(new Uint8Array(IV_BYTES));
        const data = await crypto.subtle.encrypt(
          { name: "AES-GCM", iv, additionalData: encoder.encode(name) },
          keys.seal,
          encoder.encode(JSON.stringify(value)),
        );
        const row: StoredRow = {
          name,
          kind,
          dataset: dataset === undefined ? "" : await nameOf("dataset", dataset),
          iv,
          data,
        };
        const writing = database.transaction(ROWS, "readwrite");
        writing.objectStore(ROWS).put(row);
        await committed(writing);
      }, undefined),
    get: async (kind, id) =>
      quietly(async () => {
        const name = await nameOf("row", kind, id);
        const row = await done<StoredRow | undefined>(
          database.transaction(ROWS).objectStore(ROWS).get(name),
        );
        return row === undefined ? undefined : openedRow(row);
      }, undefined),
    all: async (kind) =>
      quietly(async () => {
        const rows = await done<StoredRow[]>(
          database.transaction(ROWS).objectStore(ROWS).index(BY_KIND).getAll(kind),
        );
        return Promise.all(rows.map(openedRow));
      }, []),
    forgetDataset: async (dataset) =>
      quietly(async () => {
        const named = await nameOf("dataset", dataset);
        const writing = database.transaction(ROWS, "readwrite");
        const rows = writing.objectStore(ROWS);
        const names = await done(rows.index(BY_DATASET).getAllKeys(named));
        for (const name of names) {
          rows.delete(name);
        }
        await committed(writing);
      }, undefined),
  };
}

/**
 * Opens the cache of the page's sign-in, its key from `keys`: the key the
 * stored wrapped form holds, else a new one, after emptying a cache whose
 * key no longer unwraps. None when the browser keeps no IndexedDB or the
 * Daemon gives no key, and then nothing is written.
 */
export async function openOfflineCache(
  indexedDB: IDBFactory,
  keys: OfflineKeys,
): Promise<OfflineCache | undefined> {
  let database: IDBDatabase;
  try {
    database = await opened(indexedDB);
  } catch {
    return undefined;
  }
  try {
    const stored = await done<StoredKey | undefined>(
      database.transaction(KEY_STORE).objectStore(KEY_STORE).get(KEY_RECORD),
    );
    const unwrapped = stored === undefined ? undefined : await keys.unwrap(stored.wrapped);
    if (unwrapped !== undefined) {
      return cacheOf(database, await keysOf(unwrapped));
    }
    const emptying = database.transaction([KEY_STORE, ROWS], "readwrite");
    emptying.objectStore(KEY_STORE).clear();
    emptying.objectStore(ROWS).clear();
    await committed(emptying);
    const created = await keys.create();
    if (created === undefined) {
      database.close();
      await wipeOfflineCache(indexedDB);
      return undefined;
    }
    const keeping = database.transaction(KEY_STORE, "readwrite");
    const record: StoredKey = { id: KEY_RECORD, wrapped: created.wrapped };
    keeping.objectStore(KEY_STORE).put(record);
    await committed(keeping);
    return cacheOf(database, await keysOf(created.key));
  } catch {
    database.close();
    return undefined;
  }
}

/** Deletes the cache and everything it kept, as sign-out does. */
export async function wipeOfflineCache(indexedDB: IDBFactory): Promise<void> {
  const deleting = indexedDB.deleteDatabase(OFFLINE_DATABASE);
  await new Promise<void>((resolve) => {
    // Every open connection closes on the version change the deletion
    // sends, so a blocked deletion goes on once they have.
    deleting.addEventListener("success", () => resolve());
    deleting.addEventListener("error", () => resolve());
  });
}

/** The bytes of unpadded base64url `text`. */
function bytesOf(text: string): Uint8Array {
  const binary = atob(text.replaceAll("-", "+").replaceAll("_", "/"));
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}

/** The string field `name` of `value`, if it has one. */
function textField(value: unknown, name: string): string | undefined {
  if (typeof value === "object" && value !== null && name in value) {
    const field: unknown = Object.entries(value).find(([key]) => key === name)?.[1];
    return typeof field === "string" ? field : undefined;
  }
  return undefined;
}

/**
 * The Daemon's keys for the page's sign-in: `POST /offline-key` on the
 * Console's `origin`, with the sign-in's CSRF token. A refusal, such as an
 * ended sign-in's `401` or another sign-in's wrapped key, answers none.
 */
export function daemonOfflineKeys(
  origin: string,
  csrfToken: string,
  fetcher: typeof fetch,
): OfflineKeys {
  const asked = async (body: object): Promise<NewOfflineKey | undefined> => {
    let response: Response;
    try {
      response = await fetcher(new URL("/offline-key", origin), {
        method: "POST",
        credentials: "same-origin",
        headers: { "Content-Type": "application/json", "X-CSRF-Token": csrfToken },
        body: JSON.stringify(body),
      });
    } catch {
      return undefined;
    }
    if (!response.ok) {
      return undefined;
    }
    const answer: unknown = await response.json();
    const key = textField(answer, "key");
    const wrapped = textField(answer, "wrapped");
    return key === undefined || wrapped === undefined ? undefined : { key: bytesOf(key), wrapped };
  };
  return {
    create: async () => asked({}),
    unwrap: async (wrapped) => (await asked({ wrapped }))?.key,
  };
}
