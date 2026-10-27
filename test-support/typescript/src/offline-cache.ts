// What a served test reads of the Console's offline cache (spec #592's
// `offline`; ADR 0031) from inside its page: what IndexedDB stores, what
// the stored rows hold once decrypted with the key the page's own sign-in
// unwraps, and the revocation of that sign-in, as from another device.
// Collie's and Foxhound's served tests share them. Imported from Playwright
// tests only: it pulls in no Vitest.
import type { Page } from "@playwright/test";
import { command } from "./daemon-calls.ts";
import type { ServedDaemon } from "./served-daemon.ts";

/** The cache's IndexedDB database (`console/collie/src/offline/offline.ts`). */
export const OFFLINE_DATABASE = "goodfetch-console-offline";

/** What the cache's database holds: whether it exists, its rows' kinds, and every record as text, its bytes in base64. */
export async function storedOfflineCache(page: Page): Promise<{
  readonly exists: boolean;
  readonly kinds: string[];
  readonly text: string;
}> {
  return page.evaluate(async (name) => {
    const names = (await indexedDB.databases()).map((database) => database.name);
    if (!names.includes(name)) {
      return { exists: false, kinds: [], text: "" };
    }
    const database = await new Promise<IDBDatabase>((resolve, reject) => {
      const opening = indexedDB.open(name);
      opening.addEventListener("success", () => resolve(opening.result));
      opening.addEventListener("error", () => reject(new Error("the cache did not open")));
    });
    const records: unknown[] = [];
    for (const store of database.objectStoreNames) {
      const all = database.transaction(store).objectStore(store).getAll();
      records.push(
        ...(await new Promise<unknown[]>((resolve, reject) => {
          all.addEventListener("success", () => resolve(all.result));
          all.addEventListener("error", () => reject(new Error("the cache did not read")));
        })),
      );
    }
    database.close();
    // oxlint-disable-next-line unicorn/consistent-function-scoping -- it runs in the page, which sees nothing of the test's scope.
    const base64 = (bytes: ArrayBuffer | ArrayBufferView): string =>
      btoa(
        String.fromCharCode(...new Uint8Array(bytes instanceof ArrayBuffer ? bytes : bytes.buffer)),
      );
    const kinds = records.flatMap((record) =>
      typeof record === "object" && record !== null && "kind" in record
        ? [String(record.kind)]
        : [],
    );
    const text = JSON.stringify(records, (_, value: unknown) =>
      value instanceof ArrayBuffer || ArrayBuffer.isView(value) ? base64(value) : value,
    );
    return { exists: true, kinds, text };
  }, OFFLINE_DATABASE);
}

/**
 * What the cache's rows hold once decrypted, as the page's own sign-in can
 * read them: the key the stored wrapped form holds, from `POST
 * /offline-key`, opening each row. With the Daemon's refusal, its status.
 */
export async function decryptedOfflineCache(
  page: Page,
): Promise<{ readonly status: number; readonly rows: string[] }> {
  return page.evaluate(async (name) => {
    const database = await new Promise<IDBDatabase>((resolve, reject) => {
      const opening = indexedDB.open(name);
      opening.addEventListener("success", () => resolve(opening.result));
      opening.addEventListener("error", () => reject(new Error("the cache did not open")));
    });
    const read = async (store: string): Promise<unknown[]> =>
      new Promise((resolve, reject) => {
        const all = database.transaction(store).objectStore(store).getAll();
        all.addEventListener("success", () => resolve(all.result));
        all.addEventListener("error", () => reject(new Error("the cache did not read")));
      });
    const [keyRecord] = await read("key");
    const rows = await read("rows");
    database.close();
    const signedIn = await fetch("/session");
    if (!signedIn.ok) {
      return { status: signedIn.status, rows: [] };
    }
    const session: unknown = await signedIn.json();
    const csrf =
      typeof session === "object" && session !== null && "csrf_token" in session
        ? String(session.csrf_token)
        : "";
    const wrapped =
      typeof keyRecord === "object" && keyRecord !== null && "wrapped" in keyRecord
        ? String(keyRecord.wrapped)
        : "";
    const answer = await fetch("/offline-key", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-CSRF-Token": csrf },
      body: JSON.stringify({ wrapped }),
    });
    if (!answer.ok) {
      return { status: answer.status, rows: [] };
    }
    const body: unknown = await answer.json();
    const key = typeof body === "object" && body !== null && "key" in body ? String(body.key) : "";
    const raw = Uint8Array.from(atob(key.replaceAll("-", "+").replaceAll("_", "/")), (c) =>
      c.charCodeAt(0),
    );
    const seal = await crypto.subtle.importKey("raw", raw, "AES-GCM", false, ["decrypt"]);
    const plain: string[] = [];
    for (const row of rows) {
      if (
        typeof row === "object" &&
        row !== null &&
        "iv" in row &&
        "data" in row &&
        "name" in row
      ) {
        const iv = row.iv instanceof Uint8Array ? new Uint8Array(row.iv) : new Uint8Array();
        const data = row.data instanceof ArrayBuffer ? row.data : new ArrayBuffer(0);
        const opened = await crypto.subtle.decrypt(
          { name: "AES-GCM", iv, additionalData: new TextEncoder().encode(String(row.name)) },
          seal,
          data,
        );
        plain.push(new TextDecoder().decode(opened));
      }
    }
    return { status: answer.status, rows: plain };
  }, OFFLINE_DATABASE);
}

/** The field `name` of `value`, if it is an object that has one. */
function field(value: unknown, name: string): unknown {
  return typeof value === "object" && value !== null
    ? Object.entries(value).find(([key]) => key === name)?.[1]
    : undefined;
}

/**
 * Revokes the page's own sign-in through the Console's protocol, as the
 * person does from another device, and answers `sign_in.revoke`'s reply.
 */
export async function revokeOwnSignIn(page: Page, daemon: ServedDaemon): Promise<string> {
  const info: unknown = JSON.parse(await command(daemon, ["engine", "info", "--json"]));
  return page.evaluate(
    async ({ productVersion, needs }) => {
      const socket = new WebSocket(`ws://${window.location.host}/protocol`);
      const replies = new Map<number, (value: unknown) => void>();
      socket.addEventListener("message", (event) => {
        const message: unknown = JSON.parse(String(event.data));
        if (typeof message === "object" && message !== null && "id" in message) {
          replies.get(Number(message.id))?.(message);
        }
      });
      await new Promise((resolve) => socket.addEventListener("open", resolve));
      const call = async (id: number, method: string, params: unknown): Promise<unknown> =>
        new Promise((resolve) => {
          replies.set(id, resolve);
          socket.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
        });
      await call(0, "hello", {
        protocol_version: "0.1",
        surface: "console",
        form: "catalog",
        needs,
        product_version: productVersion,
      });
      const listed: unknown = await call(1, "sign_in.list", {});
      const id = /"id":"([0-9a-f]+)"/.exec(JSON.stringify(listed))?.[1] ?? "";
      const answer = JSON.stringify(await call(2, "sign_in.revoke", { id }));
      socket.close();
      return answer;
    },
    { productVersion: String(field(info, "product_version")), needs: daemon.product },
  );
}
