// The `catalog` family's product pages of a running Proving ground, read
// as any tool reads them: the task list, a task's start page, and the
// product it shows, which a test submits to the oracle as the task's Record.
import { request } from "node:http";

/** A running Proving ground, by its address. */
export interface ProvingGroundAddress {
  readonly address: string;
}

/** One HTTP exchange with the copy, naming `host` in the `Host` header. */
export function exchange(
  provingGround: ProvingGroundAddress,
  method: "GET" | "POST",
  host: string,
  path: string,
  body?: unknown,
): Promise<string> {
  const [hostname, port] = provingGround.address.split(":");
  return new Promise((resolve, reject) => {
    const sent = request(
      { method, hostname, port, path, headers: { host, "content-type": "application/json" } },
      (response) => {
        let text = "";
        response.setEncoding("utf8");
        response.on("data", (chunk: string) => {
          text += chunk;
        });
        response.on("end", () => {
          resolve(text);
        });
      },
    );
    sent.on("error", reject);
    sent.end(body === undefined ? undefined : JSON.stringify(body));
  });
}

export interface Task {
  readonly id: string;
  readonly start_url: string;
}

export interface TaskList {
  readonly proving_ground: { readonly version: string; readonly seed: number };
  readonly tasks: readonly Task[];
}

/** The product a `catalog` product page shows, read as any tool would. */
export function product(page: string): { id: string; name: string; price: string } {
  const field = (pattern: RegExp): string => {
    const found = pattern.exec(page)?.[1];
    if (found === undefined) {
      throw new Error(`the product page has no match for ${pattern}`);
    }
    return found;
  };
  return {
    id: field(/<article class="product" data-id="([^"]+)"/),
    name: field(/<h1 class="name">([^<]+)<\/h1>/),
    price: field(/data-price="([^"]+)"/),
  };
}

/** The product of the task `id`'s start page: the Record the oracle verifies for a `catalog` product task. */
export async function catalogRecord(
  provingGround: ProvingGroundAddress,
  id: string,
): Promise<{ id: string; name: string; price: string }> {
  const list: TaskList = JSON.parse(await exchange(provingGround, "GET", "tasks", "/tasks"));
  const task = list.tasks.find((one) => one.id === id);
  if (task === undefined) {
    throw new Error(`the task list has no ${id}`);
  }
  const url = new URL(task.start_url);
  return product(await exchange(provingGround, "GET", url.host, url.pathname));
}
