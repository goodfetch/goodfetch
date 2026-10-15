// Extracts one book from books.toscrape.com as a typed object, read by a language model.
//
// Usage: node 02-typed-extract.ts <model>      (or set GOODFETCH_MODEL)
//
// <model> is a Model reference, "<provider>/<model id>": "anthropic/claude-sonnet-4-5" for a hosted
// model, "ollama/qwen3:8b" or "lmstudio/<id>" for one served locally. Both go through the same
// `ai.model` setting; a local provider needs no key and sits at its server's loopback address.

import { Collie } from "@goodfetch/collie";
import type { ModelReference, ModelUsage } from "@goodfetch/collie";
import { z } from "zod";

const URL_OF_BOOK = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";

const Rating = z.union([z.literal(1), z.literal(2), z.literal(3), z.literal(4), z.literal(5)]);

const BookSchema = z.object({
  title: z.string().min(1).describe("The book's title, as the page's main heading shows it."),
  price: z.number().nonnegative().describe("The price in pounds as a plain decimal number, without the currency sign, such as 51.77."),
  inStock: z.int().nonnegative().describe("The number of copies in stock, as an integer: the figure in the availability line, such as 22."),
  rating: Rating.describe("The star rating as a whole number from 1 to 5 (the page marks it with the words One to Five)."),
  description: z.string().min(1).describe("The book's description text, from the product description section."),
});

type Book = z.output<typeof BookSchema>;

const INSTRUCTION =
  "Read the book shown on this product page. Give its title, its price in pounds as a decimal number, " +
  "the number of copies in stock as an integer, its star rating from 1 to 5, and its description text. " +
  "Leave a field out when the page does not show it; never guess a value.";

function parseModel(raw: string | undefined): ModelReference {
  if (raw === undefined || !/^[^/\s]+\/\S+$/.test(raw)) {
    throw new Error(
      `Name the model as "<provider>/<model id>", as an argument or in GOODFETCH_MODEL ` +
        `(for example "anthropic/claude-sonnet-4-5" or "ollama/qwen3:8b"); got ${raw === undefined ? "nothing" : JSON.stringify(raw)}.`,
    );
  }
  return raw as ModelReference;
}

function describeUsage(usage: ModelUsage): string {
  const lines = [
    `calls:         ${usage.calls}`,
    `input tokens:  ${usage.inputTokens} (cached reads ${usage.cacheReadTokens}, cache writes ${usage.cacheWriteTokens})`,
    `output tokens: ${usage.outputTokens} (reasoning ${usage.reasoningTokens})`,
    `total tokens:  ${usage.tokens}`,
    // A model with no price (a local one, or one the price table lacks) has a null price: say so
    // rather than print a made-up zero.
    usage.price === null
      ? "cost:          unknown (no price for this model; a local model costs nothing per token)"
      : `cost:          ${usage.price.amount} ${usage.price.currency}`,
  ];
  for (const m of usage.byModel) {
    lines.push(
      `  ${m.model}: ${m.tokens} tokens in ${m.calls} call(s), ` +
        (m.price === null ? "no price" : `${m.price.amount} ${m.price.currency}`),
    );
  }
  return lines.join("\n");
}

function pathText(path: readonly (string | number)[]): string {
  return path.length === 0 ? "(the whole object)" : path.map(String).join(".");
}

interface ErrorLike {
  readonly code: string;
  readonly message: string;
  readonly details?: unknown;
  readonly usage?: ModelUsage;
  readonly fix?: string;
}

function isGoodfetchError(e: unknown): e is ErrorLike {
  return (
    typeof e === "object" &&
    e !== null &&
    "code" in e &&
    typeof e.code === "string" &&
    "group" in e &&
    "message" in e &&
    typeof e.message === "string"
  );
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function pathsOf(list: unknown, key: "missing" | "issues"): { path: readonly (string | number)[]; note: string }[] {
  if (!isRecord(list)) return [];
  const items = list[key];
  if (!Array.isArray(items)) return [];
  const out: { path: readonly (string | number)[]; note: string }[] = [];
  for (const item of items as unknown[]) {
    if (!isRecord(item) || !Array.isArray(item.path)) continue;
    const path = (item.path as unknown[]).filter((p): p is string | number => typeof p === "string" || typeof p === "number");
    const note = typeof item.message === "string" ? item.message : typeof item.reason === "string" ? item.reason : "";
    out.push({ path, note });
  }
  return out;
}

/** Says which fields failed and why, for the extraction's two failures; any other error is reported as it is. */
function reportFailure(e: unknown): void {
  if (!isGoodfetchError(e)) {
    console.error(e instanceof Error ? e.message : String(e));
    return;
  }
  if (e.code === "extraction_incomplete") {
    console.error("The page did not give every field:");
    for (const m of pathsOf(e.details, "missing")) {
      console.error(`  field "${pathText(m.path)}" not found${m.note ? ` (${m.note})` : ""}`);
    }
  } else if (e.code === "extraction_invalid") {
    console.error("The model's answer does not have the expected types:");
    for (const i of pathsOf(e.details, "issues")) {
      console.error(`  field "${pathText(i.path)}": ${i.note}`);
    }
  } else {
    console.error(`${e.code}: ${e.message}`);
    if (e.fix !== undefined) console.error(`  fix: ${e.fix}`);
  }
  // A failed call's spend is never hidden.
  if (e.usage !== undefined) console.error(`\nTokens used before the failure:\n${describeUsage(e.usage)}`);
}

async function main(): Promise<Book> {
  const model = parseModel(process.argv[2] ?? process.env.GOODFETCH_MODEL);

  await using collie = await Collie.connect({ settings: { ai: { model } } });
  await using run = await collie.start();
  await using page = await run.newPage();

  await page.goto(URL_OF_BOOK);

  // "model" mode: the model reads the page at this call and every value is checked by Grounding
  // against the page; the schema's types are checked again here. A field the page lacks throws
  // ExtractionIncompleteError, a wrong type ExtractionInvalidError, each naming the field.
  const result = await page.extract(INSTRUCTION, BookSchema, { mode: "model" });

  const book = result.output;
  console.log(JSON.stringify(book, null, 2));
  console.log(`\nTokens and cost (${model}):\n${describeUsage(result.usage)}`);

  await run.finish();
  return book;
}

try {
  await main();
} catch (e) {
  reportFailure(e);
  process.exitCode = 1;
}
