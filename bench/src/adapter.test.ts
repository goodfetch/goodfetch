// The adapter protocol against a real child process: a few lines of Node that
// answer `hello` and `prepare`, echo `run`'s parameters after their `delay`,
// never answer one that asks to `hang`, answer `fail` with an error and
// `garble` with a line that is not a message.
import { describe, expect, test } from "vitest";
import { AdapterClient, AdapterSlot, type AdapterCommand } from "./adapter.ts";
import { errorCode } from "./errors.ts";

const script = `
require("node:readline").createInterface({ input: process.stdin }).on("line", (line) => {
  const m = JSON.parse(line);
  const send = (body) => process.stdout.write(JSON.stringify({ version: 1, id: m.id, ...body }) + "\\n");
  if (m.method === "hello") return send({ result: { capabilities: ["extract"], max_concurrency: 4 } });
  if (m.method === "close") { send({ result: {} }); return setTimeout(() => process.exit(0), 10); }
  if (m.method !== "run") return send({ result: {} });
  if (m.params.hang) return;
  if (m.params.fail) return send({ error: "no such page" });
  if (m.params.garble) return process.stdout.write("not json\\n");
  setTimeout(() => send({ result: m.params }), m.params.delay ?? 0);
});`;

const adapter: AdapterCommand = {
  command: [process.execPath, "-e", script],
  cwd: process.cwd(),
  timeoutMs: 5000,
};

async function codeOf(call: Promise<unknown>): Promise<string> {
  try {
    await call;
    return "answered";
  } catch (error) {
    return errorCode(error);
  }
}

describe("AdapterClient", () => {
  test("answers a call with the adapter's result, and its error or garbled line by code", async () => {
    const client = new AdapterClient(adapter);
    try {
      expect(await client.call("run", { task: "catalog/2" })).toEqual({ task: "catalog/2" });
      expect(await codeOf(client.call("run", { fail: true }))).toBe("adapter_error");
      expect(await codeOf(client.call("run", { garble: true }))).toBe("adapter_protocol");
      expect(await codeOf(client.call("run", {}))).toBe("adapter_closed");
    } finally {
      await client.close();
    }
  });

  test("times out only the stuck call and aborts its siblings", async () => {
    const client = new AdapterClient(adapter);
    try {
      const siblings = [1, 2, 3].map((index) => codeOf(client.call("run", { index, delay: 5000 })));
      const stuck = codeOf(client.call("run", { hang: true }, 200));

      expect({ stuck: await stuck, siblings: await Promise.all(siblings) }).toEqual({
        stuck: "adapter_timeout",
        siblings: ["sibling_abort", "sibling_abort", "sibling_abort"],
      });
    } finally {
      await client.close();
    }
  });
});

test("restarts a stopped adapter as a new generation, up to the campaign's limit", async () => {
  const pids: number[] = [];
  const slot = new AdapterSlot({
    adapter,
    prepare: {},
    maxRestarts: 1,
    concurrency: 2,
    started: (pid) => pids.push(pid),
  });
  try {
    const first = await slot.ensure();
    await codeOf(first.call("run", { hang: true }, 100));
    const second = await slot.ensure();
    await codeOf(second.call("run", { hang: true }, 100));

    expect({
      generation: slot.generation,
      capabilities: slot.capabilities,
      processes: new Set(pids).size,
      third: await codeOf(slot.ensure()),
    }).toEqual({ generation: 1, capabilities: ["extract"], processes: 2, third: "restart_limit" });
  } finally {
    await slot.close();
  }
});
