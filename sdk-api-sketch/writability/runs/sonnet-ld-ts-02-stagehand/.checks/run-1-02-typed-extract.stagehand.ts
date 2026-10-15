// Extract a typed book record from a product page with a language model (Stagehand 4.2.1).
//
// Usage:  node 02-typed-extract.stagehand.ts <model>      (or MODEL=<model>)
//   hosted model:  openai/gpt-4.1-mini, anthropic/claude-sonnet-4-5, google/..., groq/..., cerebras/...
//   local model:   local/<name>   served by any OpenAI-compatible endpoint (Ollama, llama.cpp, LM Studio, vLLM)
//                  endpoint: LOCAL_LLM_BASE_URL (default http://localhost:11434/v1), key: LOCAL_LLM_API_KEY (optional)
// Cost: USD per million tokens from PRICE_INPUT_PER_MTOK and PRICE_OUTPUT_PER_MTOK (a local model costs nothing).

import { Stagehand, localBrowser } from "@browserbasehq/stagehand";
import type { ClientLLM, ModelName } from "@browserbasehq/stagehand";
import { z } from "zod";

const URL_TO_READ = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";

// ---------------------------------------------------------------- the typed result

type Rating = 1 | 2 | 3 | 4 | 5;

interface Book {
  title: string;
  price: number; // pounds, decimal
  inStock: number; // copies, integer
  rating: Rating;
  description: string;
}

// What the model is asked for. Fields are nullable so that a missing field comes back as null
// (reported by name below) rather than as an invented value.
const extractionSchema = z.object({
  title: z.string().nullable().describe("The book's title, as shown in the page heading"),
  price: z
    .number()
    .nullable()
    .describe("The price in pounds as a decimal number, without the currency sign (for example 12.34)"),
  inStock: z
    .number()
    .nullable()
    .describe("The number of copies in stock, an integer taken from the availability, e.g. 'In stock (7 available)' is 7"),
  rating: z
    .number()
    .nullable()
    .describe(
      "The star rating as an integer from 1 to 5. The page encodes it in the class of the star-rating element, e.g. 'star-rating Three' is 3",
    ),
  description: z.string().nullable().describe("The product description paragraph"),
});

const INSTRUCTION =
  "Read this book's product page and return its title, price in pounds (decimal number), " +
  "number of copies in stock (integer), star rating (integer 1 to 5) and description. " +
  "If a field is not on the page, return null for it; never guess.";

// ---------------------------------------------------------------- strict validation, field by field

class FieldError extends Error {
  readonly problems: readonly string[];
  constructor(problems: readonly string[]) {
    super(`Extraction rejected:\n${problems.map((p) => `  - ${p}`).join("\n")}`);
    this.name = "FieldError";
    this.problems = problems;
  }
}

function isMissing(value: unknown): boolean {
  return value === undefined || value === null || (typeof value === "string" && value.trim() === "");
}

function describeValue(value: unknown): string {
  return `${typeof value} ${JSON.stringify(value)}`;
}

function validateBook(raw: unknown): Book {
  if (typeof raw !== "object" || raw === null) {
    throw new FieldError([`the extraction returned ${describeValue(raw)} instead of an object (no field found)`]);
  }
  const record = raw as Record<string, unknown>;
  const problems: string[] = [];

  function check<T>(field: string, expected: string, accept: (value: unknown) => value is T): T | undefined {
    const value = record[field];
    if (isMissing(value)) {
      problems.push(`field "${field}" was not found on the page`);
      return undefined;
    }
    if (!accept(value)) {
      problems.push(`field "${field}" is not ${expected}: got ${describeValue(value)}`);
      return undefined;
    }
    return value;
  }

  const title = check("title", "a text", (v): v is string => typeof v === "string");
  const price = check(
    "price",
    "a positive decimal number of pounds",
    (v): v is number => typeof v === "number" && Number.isFinite(v) && v > 0,
  );
  const inStock = check(
    "inStock",
    "a non-negative integer",
    (v): v is number => typeof v === "number" && Number.isInteger(v) && v >= 0,
  );
  const rating = check(
    "rating",
    "an integer from 1 to 5",
    (v): v is Rating => typeof v === "number" && Number.isInteger(v) && v >= 1 && v <= 5,
  );
  const description = check("description", "a text", (v): v is string => typeof v === "string");

  if (
    problems.length > 0 ||
    title === undefined ||
    price === undefined ||
    inStock === undefined ||
    rating === undefined ||
    description === undefined
  ) {
    throw new FieldError(problems);
  }
  return { title, price, inStock, rating, description };
}

// ---------------------------------------------------------------- choosing the model by name

const HOSTED_PREFIXES = ["openai/", "anthropic/", "google/", "groq/", "cerebras/"] as const;
const LOCAL_PREFIXES = ["local/", "ollama/"] as const;

function isHostedModelName(name: string): name is ModelName {
  return HOSTED_PREFIXES.some((prefix) => name.startsWith(prefix) && name.length > prefix.length);
}

function localModelId(name: string): string | undefined {
  const prefix = LOCAL_PREFIXES.find((p) => name.startsWith(p));
  return prefix !== undefined && name.length > prefix.length ? name.slice(prefix.length) : undefined;
}

function hostedApiKey(name: ModelName): string | undefined {
  const env = process.env;
  if (name.startsWith("openai/")) return env.OPENAI_API_KEY;
  if (name.startsWith("anthropic/")) return env.ANTHROPIC_API_KEY;
  if (name.startsWith("google/")) return env.GOOGLE_GENERATIVE_AI_API_KEY ?? env.GOOGLE_API_KEY;
  if (name.startsWith("groq/")) return env.GROQ_API_KEY;
  return env.CEREBRAS_API_KEY;
}

type GenerateRequest = Parameters<ClientLLM["generate"]>[0];
type GenerateResponse = Awaited<ReturnType<ClientLLM["generate"]>>;

interface ChatCompletion {
  choices?: { message?: { content?: string | null }; finish_reason?: string | null }[];
  usage?: { prompt_tokens?: number; completion_tokens?: number; total_tokens?: number };
  error?: { message?: string };
}

type OpenAiPart = { type: "text"; text: string } | { type: "image_url"; image_url: { url: string } };

// A model served locally, reached through an OpenAI-compatible chat completions endpoint.
function localClientLlm(modelId: string): ClientLLM {
  const baseUrl = (process.env.LOCAL_LLM_BASE_URL ?? "http://localhost:11434/v1").replace(/\/+$/, "");
  const apiKey = process.env.LOCAL_LLM_API_KEY;

  return {
    async generate(request: GenerateRequest): Promise<GenerateResponse> {
      const messages: { role: string; content: string | OpenAiPart[] }[] = [];
      if (request.systemPrompt !== undefined) messages.push({ role: "system", content: request.systemPrompt });
      for (const message of request.messages) {
        const blocks = typeof message.content === "object" && !Array.isArray(message.content) ? [message.content] : message.content;
        const parts: OpenAiPart[] = [];
        if (typeof blocks === "string") {
          parts.push({ type: "text", text: blocks });
        } else {
          for (const block of blocks) {
            if (block.type === "text") parts.push({ type: "text", text: block.text });
            else if (block.type === "image") {
              parts.push({ type: "image_url", image_url: { url: `data:${block.mimeType};base64,${block.data}` } });
            }
          }
        }
        messages.push({ role: message.role, content: parts });
      }

      const format = request.responseFormat;
      const structured = format !== undefined && format.type === "json_schema" ? format : undefined;

      const body: Record<string, unknown> = { model: modelId, messages, stream: false };
      if (request.temperature !== undefined) body.temperature = request.temperature;
      if (request.stopSequences !== undefined) body.stop = request.stopSequences;
      if (structured !== undefined) {
        body.response_format = {
          type: "json_schema",
          json_schema: { name: structured.name, description: structured.description, schema: structured.schema },
        };
      }

      const response = await fetch(`${baseUrl}/chat/completions`, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          ...(apiKey !== undefined ? { authorization: `Bearer ${apiKey}` } : {}),
        },
        body: JSON.stringify(body),
      });
      const payload = (await response.json()) as ChatCompletion;
      if (!response.ok) {
        throw new Error(`local model endpoint ${baseUrl} answered ${response.status}: ${payload.error?.message ?? "no detail"}`);
      }
      const choice = payload.choices?.[0];
      const text = choice?.message?.content ?? "";
      const inputTokens = payload.usage?.prompt_tokens ?? 0;
      const outputTokens = payload.usage?.completion_tokens ?? 0;
      const usage = { inputTokens, outputTokens, totalTokens: payload.usage?.total_tokens ?? inputTokens + outputTokens };
      const stopReason = choice?.finish_reason ?? undefined;

      if (structured !== undefined) {
        const json = text.trim().replace(/^```(?:json)?\s*/i, "").replace(/\s*```$/, "");
        let parsed: unknown;
        try {
          parsed = JSON.parse(json);
        } catch {
          throw new Error(`local model ${modelId} did not return JSON: ${text.slice(0, 200)}`);
        }
        return {
          role: "assistant",
          content: [{ type: "text", text }],
          stopReason,
          usage,
          outputFormat: "json_schema",
          structuredContent: parsed as Extract<GenerateResponse, { outputFormat: "json_schema" }>["structuredContent"],
        };
      }
      return { role: "assistant", content: [{ type: "text", text }], stopReason, usage, outputFormat: "text" };
    },
  };
}

// ---------------------------------------------------------------- tokens and cost

interface Tokens {
  input: number;
  output: number;
  reasoning: number;
  cachedInput: number;
}

function priceFromEnv(name: string): number | undefined {
  const raw = process.env[name];
  if (raw === undefined || raw.trim() === "") return undefined;
  const value = Number(raw);
  if (!Number.isFinite(value) || value < 0) throw new Error(`${name} must be a non-negative number, got "${raw}"`);
  return value;
}

function reportUsage(tokens: Tokens, isLocal: boolean): void {
  console.log("Tokens used");
  console.log(`  input:     ${tokens.input}${tokens.cachedInput > 0 ? ` (${tokens.cachedInput} cached)` : ""}`);
  console.log(`  output:    ${tokens.output}${tokens.reasoning > 0 ? ` (${tokens.reasoning} reasoning)` : ""}`);
  console.log(`  total:     ${tokens.input + tokens.output}`);
  if (isLocal) {
    console.log("Cost: USD 0.000000 (model served locally)");
    return;
  }
  const inputPrice = priceFromEnv("PRICE_INPUT_PER_MTOK");
  const outputPrice = priceFromEnv("PRICE_OUTPUT_PER_MTOK");
  if (inputPrice === undefined || outputPrice === undefined) {
    console.log("Cost: unknown, set PRICE_INPUT_PER_MTOK and PRICE_OUTPUT_PER_MTOK (USD per million tokens)");
    return;
  }
  const cost = (tokens.input * inputPrice + tokens.output * outputPrice) / 1_000_000;
  console.log(`Cost: USD ${cost.toFixed(6)} (at ${inputPrice} in / ${outputPrice} out per million tokens)`);
}

// ---------------------------------------------------------------- main

async function main(): Promise<void> {
  const modelName = process.argv[2] ?? process.env.MODEL;
  if (modelName === undefined || modelName === "") {
    throw new Error("Choose the model by name: pass it as the first argument or set MODEL (hosted: openai/..., anthropic/...; local: local/<name>)");
  }

  const localId = localModelId(modelName);
  let model: ClientLLM | { modelName: ModelName; apiKey?: string };
  if (localId !== undefined) {
    model = localClientLlm(localId);
  } else if (isHostedModelName(modelName)) {
    const apiKey = hostedApiKey(modelName);
    model = apiKey !== undefined ? { modelName, apiKey } : { modelName };
  } else {
    throw new Error(
      `Unknown model "${modelName}": use a hosted name (${HOSTED_PREFIXES.join(", ")}) or a local one (${LOCAL_PREFIXES.join(", ")})`,
    );
  }

  const browser = await localBrowser.launch({ headless: true });
  const stagehand = await Stagehand.create({ browser, model, logging: { level: "error" } });
  try {
    const page = await browser.context.newPage();
    await page.goto(URL_TO_READ);

    const result = await stagehand.extract(INSTRUCTION, extractionSchema, { page });
    const book = validateBook(result.data);

    console.log(`Model: ${modelName}`);
    console.log(JSON.stringify(book, null, 2));

    let tokens: Tokens = {
      input: result.metadata.usage.inputTokens,
      output: result.metadata.usage.outputTokens,
      reasoning: result.metadata.usage.reasoningTokens,
      cachedInput: result.metadata.usage.cachedInputTokens,
    };
    if (tokens.input === 0 && tokens.output === 0) {
      const metrics = await stagehand.metrics();
      tokens = {
        input: metrics.extractPromptTokens,
        output: metrics.extractCompletionTokens,
        reasoning: metrics.extractReasoningTokens,
        cachedInput: metrics.extractCachedInputTokens,
      };
    }
    reportUsage(tokens, localId !== undefined);
  } finally {
    await stagehand.close();
  }
}

try {
  await main();
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
}
