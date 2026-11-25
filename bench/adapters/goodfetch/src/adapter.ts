// The Goodfetch adapter (ADR 0025): a bench adapter that drives one Product
// through its own public SDK only, the smallest that carries the task's
// kind (ADR 0094): Collie for `extract`, Foxhound for `crawl`. It speaks
// the adapter protocol on its stdin and stdout, names its Product in its
// answer to `hello`, and answers `prepare` with the settings its row sets,
// the marks the run record keeps (ADR 0023). Each `run` starts a Run, or a
// crawl, with those settings and the ones that reach the Proving ground.
// The Product's engine is the one the process's environment names, as for
// any program of its SDK. Of the Proving ground's families it has
// programs for the catalog's; any other task answers `unsupported`.
//
// `--product collie|foxhound` picks the Product.
import { createInterface } from "node:readline";
import { parseArgs } from "node:util";
import { Collie, GoodfetchError } from "@goodfetch/collie";
import { Foxhound, seed, type DatasetRecord } from "@goodfetch/foxhound";
import type { Row } from "../../../src/campaign.ts";
import { parserOf } from "../../../src/schema.ts";
import { catalog, listing, productOf, products } from "./catalog.ts";
import { nested, reachSettings, rowSettings, type FlatSettings } from "./rows.ts";

/** What a Product does with one task's start URL under its settings: the records it found. */
interface Driver {
  readonly product: "collie" | "foxhound";
  readonly kind: "extract" | "crawl";
  records(startUrl: string, settings: FlatSettings): Promise<readonly unknown[]>;
  close(): Promise<void>;
}

async function collieDriver(): Promise<Driver> {
  const collie = await Collie.connect();
  return {
    product: "collie",
    kind: "extract",
    async records(startUrl, settings) {
      const started = await collie.start({ settings: nested(settings) });
      try {
        const page = await started.newPage();
        await page.goto(startUrl);
        return [await productOf(page)];
      } finally {
        await started.finish();
      }
    },
    close: () => collie.close(),
  };
}

async function foxhoundDriver(): Promise<Driver> {
  const foxhound = await Foxhound.connect();
  return {
    product: "foxhound",
    kind: "crawl",
    async records(startUrl, settings) {
      const crawl = await foxhound.startCrawl(catalog, {
        seeds: [seed(startUrl, listing)],
        settings: nested(settings),
      });
      await crawl.finish();
      const records: DatasetRecord[] = [];
      for await (const record of crawl.records(products)) records.push(record);
      return records.map((record) => record.value);
    },
    close: () => foxhound.close(),
  };
}

/** A request of the adapter protocol, as the runner sends it. */
interface Request {
  readonly id: number;
  readonly method: string;
  readonly params?: unknown;
}

const request = parserOf<Request>({
  type: "object",
  required: ["id", "method"],
  properties: { id: { type: "integer" }, method: { type: "string" }, params: true },
});

/** What `prepare` hands the adapter. */
interface PrepareParams {
  readonly proving_ground: { readonly address: string };
  readonly row: Row;
}

const prepareParams = parserOf<PrepareParams>({
  type: "object",
  required: ["proving_ground", "row"],
  properties: {
    proving_ground: {
      type: "object",
      required: ["address"],
      properties: { address: { type: "string" } },
    },
    row: {
      type: "object",
      required: ["kind"],
      oneOf: [
        { properties: { kind: { const: "default" } } },
        {
          required: ["against", "settings", "competitor_defaults"],
          properties: {
            kind: { const: "matched" },
            against: { type: "string" },
            settings: { type: "object" },
            competitor_defaults: {
              type: "object",
              required: ["filter_list_blocking", "consent_prompt_handling"],
              properties: {
                filter_list_blocking: { type: "boolean" },
                consent_prompt_handling: { type: "boolean" },
              },
            },
          },
        },
      ],
    },
  },
});

/** What `run` hands the adapter: one task. */
interface RunParams {
  readonly task: { readonly id: string; readonly kind: string; readonly start_url: string };
}

const runParams = parserOf<RunParams>({
  type: "object",
  required: ["task"],
  properties: {
    task: {
      type: "object",
      required: ["id", "kind", "start_url"],
      properties: {
        id: { type: "string" },
        kind: { type: "string" },
        start_url: { type: "string" },
      },
    },
  },
});

const { values } = parseArgs({ options: { product: { type: "string" } } });
const product = values.product;
if (product !== "collie" && product !== "foxhound") {
  throw new Error(`--product must be collie or foxhound, not ${String(product)}`);
}

let driver: Driver | undefined;
let address = "";
let applied: FlatSettings = {};

function reply(id: number, result: unknown): void {
  process.stdout.write(`${JSON.stringify({ version: 1, id, result })}\n`);
}

function refuse(id: number, error: unknown): void {
  const message = error instanceof Error ? error.message : String(error);
  process.stdout.write(`${JSON.stringify({ version: 1, id, error: message })}\n`);
}

async function prepare(params: unknown): Promise<unknown> {
  const { proving_ground, row } = prepareParams.parse(params, "prepare params");
  address = proving_ground.address;
  applied = rowSettings(row);
  driver = product === "collie" ? await collieDriver() : await foxhoundDriver();
  return { settings: applied };
}

async function run(params: unknown): Promise<unknown> {
  const { task } = runParams.parse(params, "run params");
  if (driver === undefined) {
    throw new Error("run before prepare");
  }
  if (task.kind !== driver.kind || !task.id.startsWith("catalog/")) {
    return {
      status: "unsupported",
      reason: `the Goodfetch adapter has no ${driver.product} program for ${task.id} (${task.kind})`,
    };
  }
  try {
    const records = await driver.records(task.start_url, {
      ...applied,
      ...reachSettings(task.start_url, address),
    });
    return { status: "success", records };
  } catch (error) {
    const code = error instanceof GoodfetchError ? error.code : "program_error";
    return { status: "failure", records: [], error: { code } };
  }
}

async function answer(id: number, work: Promise<unknown>): Promise<void> {
  try {
    reply(id, await work);
  } catch (error) {
    refuse(id, error);
  }
}

createInterface({ input: process.stdin }).on("line", (line) => {
  const message = request.parse(JSON.parse(line), "adapter request");
  switch (message.method) {
    case "hello":
      reply(message.id, {
        capabilities: [product === "collie" ? "extract" : "crawl"],
        product,
        max_concurrency: 4,
      });
      return;
    case "prepare":
      void answer(message.id, prepare(message.params));
      return;
    case "run":
      void answer(message.id, run(message.params));
      return;
    case "close":
      void (driver?.close() ?? Promise.resolve()).finally(() => {
        process.stdout.write(
          `${JSON.stringify({ version: 1, id: message.id, result: {} })}\n`,
          () => {
            process.exit(0);
          },
        );
      });
      return;
    default:
      refuse(message.id, `unknown method ${message.method}`);
  }
});
