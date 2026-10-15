import {
  Collie,
  ExtractionIncompleteError,
  ExtractionInvalidError,
  GoodfetchError,
  type ModelReference,
  type ModelUsage,
} from "@goodfetch/collie";
import { z } from "zod";

const PAGE_URL = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";

// The typed shape of the book. Engine-side validation (JSON Schema and these validators) runs on
// every model answer, so a value that fails the schema is never returned.
const Book = z.object({
  title: z.string().min(1),
  price: z.number().positive(),
  stock: z.number().int().nonnegative(),
  rating: z.union([z.literal(1), z.literal(2), z.literal(3), z.literal(4), z.literal(5)]),
  description: z.string().min(1),
});

type BookRecord = z.infer<typeof Book>;

const INSTRUCTION = [
  "Read the book shown on this page and return:",
  "title: the book's title;",
  "price: its price in pounds as a decimal number, without the pound sign;",
  "stock: the number of copies in stock, as an integer, from the availability text;",
  "rating: its star rating, an integer from 1 to 5 (the page writes it as a word, from One to Five);",
  "description: the text of the product description.",
].join(" ");

function modelFromInput(): ModelReference {
  const name = process.argv[2] ?? process.env["GOODFETCH_MODEL"];
  if (name === undefined || !name.includes("/")) {
    throw new Error(
      "Name the model as <provider>/<model id>, for example ollama/qwen3:8b, in argv[2] or in GOODFETCH_MODEL.",
    );
  }
  return name as ModelReference;
}

function fieldOf(path: readonly (string | number)[]): string {
  return path.length > 0 ? path.join(".") : "(whole result)";
}

function explainFailure(error: GoodfetchError): string[] {
  if (error instanceof ExtractionIncompleteError) {
    return error.details.missing.map(
      (item) => `field "${fieldOf(item.path)}" was not found on the page (${item.reason})`,
    );
  }
  if (error instanceof ExtractionInvalidError) {
    return error.details.issues.map(
      (issue) => `field "${fieldOf(issue.path)}" has the wrong type or value: ${issue.message}`,
    );
  }
  return [`${error.code} during ${error.operation}: ${error.message}`];
}

function describeUsage(usage: ModelUsage): string {
  const cost =
    usage.price === null
      ? "unknown (no price for this model in the price table)"
      : `${usage.price.amount} ${usage.price.currency}`;
  return [
    `tokens: input ${usage.inputTokens}, output ${usage.outputTokens}, total ${usage.tokens}, model calls ${usage.calls}`,
    `cost: ${cost}`,
  ].join("\n");
}

async function main(): Promise<void> {
  const model = modelFromInput();
  await using collie = await Collie.connect();
  await using run = await collie.start();
  const page = await run.newPage();
  try {
    await page.goto(PAGE_URL);
    const result = await page.extract<typeof Book>(INSTRUCTION, Book, { model, mode: "model" });
    const book: BookRecord = result.output;
    await run.finish();
    console.log(JSON.stringify(book, null, 2));
    console.log(describeUsage(result.usage));
  } catch (error) {
    if (!(error instanceof GoodfetchError)) {
      throw error;
    }
    for (const line of explainFailure(error)) {
      console.error(line);
    }
    if (error.usage !== undefined) {
      console.error(describeUsage(error.usage));
    }
    process.exitCode = 1;
  }
}

await main();
