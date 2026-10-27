// The offline cache behind its interface (spec #592's `offline`; ADR 0031),
// in a real browser's IndexedDB and Web Crypto, with the Daemon's keys
// stood in for by a map of wrapped forms, as the Daemon holds one wrapping
// key per sign-in: a value put is read back while the stored records hold
// only ciphertext and opaque names; a cache whose key no longer unwraps,
// as after a revoked sign-in, is emptied before anything is written again;
// wiping removes it all; a Dataset's erasure deletes that Dataset's rows
// only; and with no key, nothing is written.
import { beforeEach, describe, expect, test } from "vitest";
import {
  OFFLINE_DATABASE,
  openOfflineCache,
  wipeOfflineCache,
  type NewOfflineKey,
  type OfflineKeys,
} from "./offline.ts";

/** Keys as one sign-in's Daemon gives them, until `revoke` ends the sign-in. */
class StandInKeys implements OfflineKeys {
  readonly #keys = new Map<string, Uint8Array>();
  #revoked = false;
  created = 0;

  async create(): Promise<NewOfflineKey | undefined> {
    if (this.#revoked) {
      return undefined;
    }
    this.created += 1;
    const key = crypto.getRandomValues(new Uint8Array(32));
    const wrapped = `wrapped-${this.created}`;
    this.#keys.set(wrapped, key);
    return Promise.resolve({ key, wrapped });
  }

  async unwrap(wrapped: string): Promise<Uint8Array | undefined> {
    return Promise.resolve(this.#revoked ? undefined : this.#keys.get(wrapped));
  }

  revoke(): void {
    this.#revoked = true;
  }
}

/** Every record of every store of the cache's database, as JSON text. */
async function storedText(): Promise<string> {
  const database = await new Promise<IDBDatabase>((resolve, reject) => {
    const opening = indexedDB.open(OFFLINE_DATABASE);
    opening.addEventListener("success", () => resolve(opening.result));
    opening.addEventListener("error", () => reject(opening.error ?? new Error("no database")));
  });
  const records: unknown[] = [];
  for (const name of database.objectStoreNames) {
    const all = database.transaction(name).objectStore(name).getAll();
    records.push(
      ...(await new Promise<unknown[]>((resolve, reject) => {
        all.addEventListener("success", () => resolve(all.result));
        all.addEventListener("error", () => reject(all.error ?? new Error("no records")));
      })),
    );
  }
  database.close();
  return JSON.stringify(records, (_, value: unknown) =>
    value instanceof ArrayBuffer || ArrayBuffer.isView(value)
      ? new TextDecoder("latin1").decode(value)
      : value,
  );
}

const SUMMARY = { run_id: "shop-crawl-17", lifecycle: "finished", note: "plain marker text" };

beforeEach(async () => {
  await wipeOfflineCache(indexedDB);
});

describe("the offline cache", () => {
  test("reads back what it keeps, while its records hold only ciphertext and opaque names", async () => {
    const keys = new StandInKeys();
    const cache = await openOfflineCache(indexedDB, keys);

    await cache?.put("run_summary", "shop-crawl-17", SUMMARY);
    const reopened = await openOfflineCache(indexedDB, keys);

    expect(await reopened?.get("run_summary", "shop-crawl-17")).toEqual(SUMMARY);
    expect(await reopened?.all("run_summary")).toEqual([SUMMARY]);
    expect(await reopened?.all("run_page")).toEqual([]);
    expect(keys.created).toBe(1);
    const stored = await storedText();
    expect(stored).not.toContain("shop-crawl-17");
    expect(stored).not.toContain("plain marker text");
  });

  test("a cache whose key no longer unwraps is emptied, and nothing is written without a key", async () => {
    const keys = new StandInKeys();
    const cache = await openOfflineCache(indexedDB, keys);
    await cache?.put("run_summary", "shop-crawl-17", SUMMARY);

    const fresh = new StandInKeys();
    const another = await openOfflineCache(indexedDB, fresh);
    expect(await another?.all("run_summary")).toEqual([]);

    fresh.revoke();
    expect(await openOfflineCache(indexedDB, fresh)).toBeUndefined();
    expect(await storedText()).toBe("[]");
  });

  test("wiping removes everything it kept", async () => {
    const keys = new StandInKeys();
    const cache = await openOfflineCache(indexedDB, keys);
    await cache?.put("run_summary", "shop-crawl-17", SUMMARY);

    await wipeOfflineCache(indexedDB);

    const databases = await indexedDB.databases();
    expect(databases.map(({ name }) => name)).not.toContain(OFFLINE_DATABASE);
  });

  test("a Dataset's erasure deletes that Dataset's rows only", async () => {
    const cache = await openOfflineCache(indexedDB, new StandInKeys());
    await cache?.put("dataset_view", "a", { n: 1 }, "products");
    await cache?.put("dataset_view", "b", { n: 2 }, "reviews");
    await cache?.put("run_summary", "c", { n: 3 });

    await cache?.forgetDataset("products");

    expect(await cache?.all("dataset_view")).toEqual([{ n: 2 }]);
    expect(await cache?.all("run_summary")).toEqual([{ n: 3 }]);
  });
});
