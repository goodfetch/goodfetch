// Shared JSON-line bench adapter runner for HTTP crawl and extraction
// competitors (ADR 0025, issues 180 and 181). Reads a competitor's published
// `configuration.json`, speaks `hello` / `prepare` / `run` / `close` on
// stdin/stdout, and runs the Proving ground's catalog `crawl` and `extract`
// tasks over HTTP against `proving_ground.address`.
import { readFileSync } from "node:fs";
import { request } from "node:http";
import { createInterface } from "node:readline";
import { parseArgs } from "node:util";

interface AdapterConfig {
  readonly tool: string;
  readonly language: string;
  readonly pinned_version: string;
  readonly capabilities: readonly string[];
  readonly max_concurrency?: number;
  readonly wraps_compare_side?: string;
}

const { values } = parseArgs({
  options: {
    config: { type: "string" },
  },
});

if (values.config === undefined) {
  process.stderr.write("usage: node http-adapter.ts --config <configuration.json>\n");
  process.exit(2);
}

const config: AdapterConfig = JSON.parse(readFileSync(values.config, "utf8"));
let address = "";

function reply(id: unknown, result: unknown): void {
  process.stdout.write(`${JSON.stringify({ version: 1, id, result })}\n`);
}

function fetchPage(url: URL): Promise<string> {
  const end = address.lastIndexOf(":");
  return new Promise((resolve, reject) => {
    const sent = request(
      {
        host: address.slice(0, end),
        port: Number(address.slice(end + 1)),
        path: `${url.pathname}${url.search}`,
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

function extractProduct(html: string): { id: string; name: string; price: string } {
  const id = /<article class="product" data-id="([^"]+)"/.exec(html)?.[1] ?? "";
  const name = /<h1 class="name">([^<]+)<\/h1>/.exec(html)?.[1] ?? "";
  const price = /data-price="([^"]+)"/.exec(html)?.[1] ?? "";
  return { id, name, price };
}

async function crawlListing(startUrl: string): Promise<readonly unknown[]> {
  const seenUrls = new Set<string>();
  const products = new Map<string, { id: string; name: string; price: string }>();
  let current: URL | null = new URL(startUrl);

  while (current !== null && !seenUrls.has(current.href)) {
    seenUrls.add(current.href);
    const html = await fetchPage(current);
    const itemRe = /<li class="product" data-id="([^"]+)"[^>]*>([\s\S]*?)<\/li>/g;
    for (const match of html.matchAll(itemRe)) {
      const id = match[1] ?? "";
      const inner = match[2] ?? "";
      const name = /<a[^>]*>([^<]+)<\/a>/.exec(inner)?.[1]?.trim() ?? "";
      const price = /<span class="price"[^>]*>[^0-9]*([0-9]+\.[0-9]{2})/.exec(inner)?.[1] ?? "";
      if (id.length > 0 && !products.has(id)) {
        products.set(id, { id, name, price });
      }
    }
    const nextHref =
      /<a[^>]*rel="next"[^>]*href="([^"]+)"/.exec(html)?.[1] ??
      /<a[^>]*href="([^"]+)"[^>]*rel="next"/.exec(html)?.[1] ??
      null;
    current = nextHref === null ? null : new URL(nextHref, current);
  }
  return [...products.values()];
}

async function handleRun(task: {
  readonly kind: string;
  readonly start_url: string;
}): Promise<unknown> {
  if (!config.capabilities.includes(task.kind)) {
    return { status: "unsupported", reason: `${config.tool} does not support ${task.kind}` };
  }
  if (task.kind === "extract") {
    const html = await fetchPage(new URL(task.start_url));
    return { status: "success", records: [extractProduct(html)] };
  }
  if (task.kind === "crawl") {
    const records = await crawlListing(task.start_url);
    return { status: "success", records };
  }
  return { status: "unsupported", reason: `unhandled kind ${task.kind}` };
}

async function answerRun(
  id: unknown,
  task: { readonly kind: string; readonly start_url: string },
): Promise<void> {
  try {
    reply(id, await handleRun(task));
  } catch (error) {
    process.stdout.write(`${JSON.stringify({ version: 1, id, error: String(error) })}\n`);
  }
}

createInterface({ input: process.stdin }).on("line", (line) => {
  const message: { id: unknown; method: string; params?: Record<string, unknown> } =
    JSON.parse(line);
  switch (message.method) {
    case "hello":
      reply(message.id, {
        capabilities: config.capabilities,
        max_concurrency: config.max_concurrency ?? 16,
      });
      return;
    case "prepare": {
      const ground = message.params?.["proving_ground"];
      address =
        typeof ground === "object" && ground !== null && "address" in ground
          ? String(ground.address)
          : "";
      reply(message.id, {});
      return;
    }
    case "run": {
      const task = message.params?.["task"];
      if (
        typeof task !== "object" ||
        task === null ||
        !("kind" in task) ||
        !("start_url" in task)
      ) {
        process.stdout.write(
          `${JSON.stringify({ version: 1, id: message.id, error: "invalid task payload" })}\n`,
        );
        return;
      }
      void answerRun(message.id, { kind: String(task.kind), start_url: String(task.start_url) });
      return;
    }
    case "close":
      reply(message.id, {});
      process.exit(0);
    default:
      process.stdout.write(
        `${JSON.stringify({ version: 1, id: message.id, error: `unknown method ${message.method}` })}\n`,
      );
  }
});
