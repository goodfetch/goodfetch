/// <reference types="node" />
// Golden scenario 02, "Extract a typed object with a chosen model" (ADRs 0082, 0091, 0101).
import { z } from "zod";
import {
  Collie,
  ExtractionIncompleteError,
  ExtractionInvalidError,
  expect,
} from "@goodfetch/collie";

const Book = z.object({
  title: z.string(),
  price: z.string().regex(/^\d+\.\d{2}$/), // pounds, an exact decimal kept as text
  inStock: z.number().int(),
  rating: z.number().int().min(1).max(5),
  description: z.string(),
});

// A Model reference names its provider, then that provider's model ID: a model Ollama serves locally
// is one more provider, priced zero at its default address (ADR 0101).
const model = process.argv.includes("--local") ? "ollama/qwen3:8b" : "anthropic/claude-sonnet-4-5";

await using collie = await Collie.connect(); // Anthropic's key: env:ANTHROPIC_API_KEY, sent only to Anthropic
await using run = await collie.start();
const page = await run.newPage();
await page.goto("https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html");
await expect(page).toHaveTitle(/A Light in the Attic/); // not another page, nor an error page
try {
  // Every answer is checked against Book; a provider that rejects native structured output gets a
  // forced tool call, then the schema in the prompt (`ai.structured_output: auto`).
  const { output: book, usage } = await page.extract("the book on this page", Book, { model });
  const price = usage.price === null ? "unpriced" : `${usage.price.amount} ${usage.price.currency}`;
  console.log(
    book,
    `${usage.tokens} tokens (${usage.inputTokens} in, ${usage.outputTokens} out), ${price}`,
  );
} catch (error) {
  if (error instanceof ExtractionIncompleteError) {
    console.error(
      "Not on the page:",
      error.details.missing.map((field) => field.path.join(".")),
    );
  }
  if (error instanceof ExtractionInvalidError) {
    console.error(
      "Not of the expected type:",
      error.details.issues.map((issue) => `${issue.path.join(".")}: ${issue.message}`),
    );
  }
  throw error; // its `usage` says what the failed attempts spent
}
await run.finish();
