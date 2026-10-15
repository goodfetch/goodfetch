/// <reference types="node" />
// Golden scenario 02 on Stagehand 4.2.1: `extract` with a Zod schema, after its README and its
// "Custom models" guide (docs.stagehand.dev/v4/configuration/models).
import { type ClientLLM, localBrowser, Stagehand } from "@browserbasehq/stagehand";
import { z } from "zod";

const Book = z.object({
  title: z.string(),
  price: z.number().describe("in pounds"),
  inStock: z.number().int(),
  rating: z.number().int().min(1).max(5),
  description: z.string(),
});

// Stagehand reports tokens, never a price: USD per million tokens, copied from Anthropic's price page.
const PRICE = { input: 3, output: 15 };

// A model name may only start with openai/, anthropic/, google/, groq/ or cerebras/: a model served
// locally is a callback the program writes against the server's own API, here Ollama's.
type GenerateParams = Parameters<ClientLLM["generate"]>[0];
type Content = GenerateParams["messages"][number]["content"];
const textOf = (content: Content) =>
  (Array.isArray(content) ? content : [content])
    .map((block) => (block.type === "text" ? block.text : ""))
    .join("\n");

async function generateWithOllama(params: GenerateParams): ReturnType<ClientLLM["generate"]> {
  if (params.responseFormat?.type !== "json_schema") {
    throw new TypeError("Stagehand only issues structured generations");
  }
  const response = await fetch("http://localhost:11434/v1/chat/completions", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      model: "qwen3:8b",
      messages: [
        { role: "system", content: params.systemPrompt ?? "" },
        ...params.messages.map((message) => ({
          role: message.role,
          content: textOf(message.content),
        })),
      ],
      temperature: params.temperature,
      response_format: {
        type: "json_schema",
        json_schema: { name: params.responseFormat.name, schema: params.responseFormat.schema },
      },
    }),
  });
  if (!response.ok) throw new Error(`Ollama answered ${response.status}: ${await response.text()}`);
  const body = (await response.json()) as {
    choices: { message: { content: string } }[];
    usage: { prompt_tokens: number; completion_tokens: number; total_tokens: number };
  };
  const text = body.choices[0]?.message.content ?? "";
  return {
    role: "assistant",
    content: { type: "text", text },
    outputFormat: "json_schema",
    structuredContent: JSON.parse(text),
    usage: {
      inputTokens: body.usage.prompt_tokens,
      outputTokens: body.usage.completion_tokens,
      totalTokens: body.usage.total_tokens,
    },
  };
}

const local = process.argv.includes("--local");
const browser = await localBrowser.launch();
const stagehand = await Stagehand.create({
  browser,
  model: local
    ? { generate: generateWithOllama }
    : { modelName: "anthropic/claude-sonnet-4-5", apiKey: process.env.ANTHROPIC_API_KEY },
});

const [page] = await browser.context.pages();
await page.goto("https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html");

// The answer is parsed with Book: a value of the wrong type throws a ZodError naming its path.
const { data: book, metadata } = await stagehand.extract("extract the book on this page", Book);
const { inputTokens, outputTokens } = metadata.usage;
const price = local ? 0 : (inputTokens * PRICE.input + outputTokens * PRICE.output) / 1_000_000;
console.log(
  book,
  `${inputTokens + outputTokens} tokens (${inputTokens} in, ${outputTokens} out), ${price.toFixed(4)} USD`,
);

await stagehand.close();
await browser.close();
