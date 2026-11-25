// A stub competitor adapter for the runner's tests: a real process that
// speaks the adapter protocol on its stdin and stdout and reads the Proving
// ground's product page as any tool would. Its arguments script its
// correctness and speed:
//
// - `--answer correct|wrong`: the product as the page shows it, or with
//   another price, both reported as a success.
// - `--delay-ms <n>`: how long it waits before answering each `run`.
// - `--hang-once`: from its second `run` on it answers nothing, once per
//   test: it leaves a mark in the folder `GOODFETCH_BENCH_STUB_STATE` names,
//   and a restarted stub that finds the mark answers.
// - `--capabilities <kind,...>`: the task kinds it claims, `extract` by default.
import { existsSync, writeFileSync } from "node:fs";
import { request } from "node:http";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { parseArgs } from "node:util";

const { values } = parseArgs({
  options: {
    answer: { type: "string", default: "correct" },
    "delay-ms": { type: "string", default: "0" },
    "hang-once": { type: "boolean", default: false },
    capabilities: { type: "string", default: "extract" },
  },
});
const delayMs = Number(values["delay-ms"]);
const hangMark =
  values["hang-once"] && process.env["GOODFETCH_BENCH_STUB_STATE"] !== undefined
    ? join(process.env["GOODFETCH_BENCH_STUB_STATE"], "hung")
    : null;

let address = "";
let runs = 0;
let hung = false;

function reply(id: unknown, result: unknown): void {
  process.stdout.write(`${JSON.stringify({ version: 1, id, result })}\n`);
}

function page(url: URL): Promise<string> {
  const end = address.lastIndexOf(":");
  return new Promise((resolve, reject) => {
    const sent = request(
      {
        host: address.slice(0, end),
        port: Number(address.slice(end + 1)),
        path: url.pathname,
        headers: { host: url.host, connection: "close" },
      },
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
    sent.end();
  });
}

function field(text: string, pattern: RegExp): string {
  return pattern.exec(text)?.[1] ?? "";
}

async function run(startUrl: string): Promise<unknown> {
  const text = await page(new URL(startUrl));
  const product = {
    id: field(text, /<article class="product" data-id="([^"]+)"/),
    name: field(text, /<h1 class="name">([^<]+)<\/h1>/),
    price: field(text, /data-price="([^"]+)"/),
  };
  const records = [values.answer === "wrong" ? { ...product, price: "0.01" } : product];
  return { status: "success", records };
}

async function answer(id: unknown, startUrl: string): Promise<void> {
  try {
    reply(id, await run(startUrl));
  } catch (error) {
    process.stdout.write(`${JSON.stringify({ version: 1, id, error: String(error) })}\n`);
  }
}

createInterface({ input: process.stdin }).on("line", (line) => {
  const message: { id: unknown; method: string; params: Record<string, unknown> } =
    JSON.parse(line);
  switch (message.method) {
    case "hello":
      reply(message.id, { capabilities: values.capabilities.split(","), max_concurrency: 16 });
      return;
    case "prepare": {
      const ground = message.params["proving_ground"];
      address =
        typeof ground === "object" && ground !== null && "address" in ground
          ? String(ground.address)
          : "";
      reply(message.id, {});
      return;
    }
    case "run": {
      runs += 1;
      if (hangMark !== null && runs === 2 && !existsSync(hangMark)) {
        writeFileSync(hangMark, "1");
        hung = true;
      }
      if (hung) {
        return;
      }
      const task = message.params["task"];
      const startUrl =
        typeof task === "object" && task !== null && "start_url" in task
          ? String(task.start_url)
          : "";
      setTimeout(() => {
        void answer(message.id, startUrl);
      }, delayMs);
      return;
    }
    case "close":
      process.stdout.write(
        `${JSON.stringify({ version: 1, id: message.id, result: {} })}\n`,
        () => {
          process.exit(0);
        },
      );
      return;
    default:
      process.stdout.write(
        `${JSON.stringify({ version: 1, id: message.id, error: `unknown method ${message.method}` })}\n`,
      );
  }
});
