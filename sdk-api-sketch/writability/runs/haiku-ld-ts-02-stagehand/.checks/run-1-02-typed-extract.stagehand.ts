// Extracts one book from books.toscrape.com as a typed object, using a language model
// to read the page. The model is chosen by name from argv[2], or the MODEL variable:
//   - "local/<model>" is served by an OpenAI-compatible server at LOCAL_MODEL_URL
//     (default http://localhost:11434/v1, Ollama). It is called through Stagehand's
//     client-side model hook, so it takes the same path as a hosted model.
//   - "<provider>/<model>" (openai, anthropic, google, groq, cerebras) is a hosted model;
//     MODEL_API_KEY, when set, is passed as its API key.
// Cost: PRICE_IN_PER_MTOK and PRICE_OUT_PER_MTOK (USD per million tokens) price hosted
// models; local models are not billed.

import { Stagehand, localBrowser } from "@browserbasehq/stagehand";
import type { ModelName, StagehandCreateOptions } from "@browserbasehq/stagehand";
import { z } from "zod";

const BOOK_URL = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html";
const DEFAULT_MODEL = "openai/gpt-4o-mini";
const LOCAL_PREFIX = "local/";
const LOCAL_URL_DEFAULT = "http://localhost:11434/v1";

const INSTRUCTION = [
  "Read this book's page and extract:",
  "title: the book's title as text;",
  "price: the price in pounds as a decimal number, without the currency sign (for example 51.77);",
  "copiesInStock: the number of copies in stock as a whole number (from text such as 'In stock (22 available)');",
  "starRating: the star rating as a whole number from 1 to 5 (the page gives it as a word, for example 'Three' is 3);",
  "description: the book's description as text.",
  "Use null for any field that is not on the page.",
].join(" ");

// Lenient shape the model fills in. A field it cannot read is null; a field of the wrong
// kind is caught by checkBook, which names it.
const RawBookSchema = z.object({
  title: z.string().nullable(),
  price: z.number().nullable(),
  copiesInStock: z.number().int().nullable(),
  starRating: z.number().int().nullable(),
  description: z.string().nullable(),
});

const BookFieldSchemas = {
  title: z.string().min(1),
  price: z.number().nonnegative(),
  copiesInStock: z.number().int().nonnegative(),
  starRating: z.literal([1, 2, 3, 4, 5]),
  description: z.string().min(1),
};

type StarRating = 1 | 2 | 3 | 4 | 5;

type Book = {
  title: string;
  price: number;
  copiesInStock: number;
  starRating: StarRating;
  description: string;
};

type CheckedBook = { ok: true; book: Book } | { ok: false; problems: string[] };

function checkField<S extends z.ZodType>(
  name: string,
  value: unknown,
  schema: S,
  expected: string,
  problems: string[],
): z.output<S> | undefined {
  if (value === null || value === undefined) {
    problems.push(`field ${name}: not found on the page`);
    return undefined;
  }
  const parsed = schema.safeParse(value);
  if (parsed.success) {
    return parsed.data;
  }
  problems.push(`field ${name}: expected ${expected}, got ${JSON.stringify(value)}`);
  return undefined;
}

function checkBook(raw: z.output<typeof RawBookSchema>): CheckedBook {
  const problems: string[] = [];
  const title = checkField("title", raw.title, BookFieldSchemas.title, "text", problems);
  const price = checkField(
    "price",
    raw.price,
    BookFieldSchemas.price,
    "a decimal number of pounds",
    problems,
  );
  const copiesInStock = checkField(
    "copiesInStock",
    raw.copiesInStock,
    BookFieldSchemas.copiesInStock,
    "a whole number of copies",
    problems,
  );
  const starRating = checkField(
    "starRating",
    raw.starRating,
    BookFieldSchemas.starRating,
    "a whole number from 1 to 5",
    problems,
  );
  const description = checkField("description", raw.description, BookFieldSchemas.description, "text", problems);

  if (
    title === undefined ||
    price === undefined ||
    copiesInStock === undefined ||
    starRating === undefined ||
    description === undefined
  ) {
    return { ok: false, problems };
  }
  return { ok: true, book: { title, price, copiesInStock, starRating, description } };
}

// The model option type, and the client-side generate function inside its ClientLLM variant.
type ModelOption = NonNullable<StagehandCreateOptions["model"]>;
type ClientModel = Extract<ModelOption, { generate: unknown }>;
type GenerateFn = ClientModel["generate"];
type LLMRequest = Parameters<GenerateFn>[0];
type LLMContent = LLMRequest["messages"][number]["content"];

type OpenAIPart = { type: "text"; text: string } | { type: "image_url"; image_url: { url: string } };
type OpenAIMessage = { role: "system" | "user" | "assistant"; content: OpenAIPart[] };

const ChatCompletion = z.object({
  choices: z.array(z.object({ message: z.object({ content: z.string().nullable() }) })),
  usage: z
    .object({
      prompt_tokens: z.number(),
      completion_tokens: z.number(),
      total_tokens: z.number().optional(),
    })
    .optional(),
});

const JsonValue = z.json();

function toOpenAIParts(content: LLMContent): OpenAIPart[] {
  const parts = Array.isArray(content) ? content : [content];
  return parts.map((part): OpenAIPart => {
    switch (part.type) {
      case "text":
        return { type: "text", text: part.text };
      case "image":
        return { type: "image_url", image_url: { url: `data:${part.mimeType};base64,${part.data}` } };
      default:
        throw new Error(`the local model cannot take a "${part.type}" content part`);
    }
  });
}

function parseJsonValue(text: string) {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    throw new Error(`the local model did not return JSON: ${text.slice(0, 200)}`);
  }
  return JsonValue.parse(value);
}

// Serves Stagehand's model calls from an OpenAI-compatible chat endpoint.
function localModel(modelName: string, baseUrl: string): ModelOption {
  const generate: GenerateFn = async (request) => {
    if ("tools" in request && request.tools !== undefined) {
      throw new Error("the local model adapter does not support tool calls");
    }
    const messages: OpenAIMessage[] = [];
    if (request.systemPrompt !== undefined) {
      messages.push({ role: "system", content: [{ type: "text", text: request.systemPrompt }] });
    }
    for (const message of request.messages) {
      messages.push({ role: message.role, content: toOpenAIParts(message.content) });
    }

    const format = request.responseFormat;
    const response = await fetch(`${baseUrl}/chat/completions`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        model: modelName,
        messages,
        temperature: request.temperature ?? 0,
        stop: request.stopSequences,
        response_format:
          format === undefined
            ? undefined
            : {
                type: "json_schema",
                json_schema: { name: format.name, description: format.description, schema: format.schema },
              },
      }),
    });
    if (!response.ok) {
      const detail = (await response.text()).slice(0, 300);
      throw new Error(`local model at ${baseUrl} answered HTTP ${response.status}: ${detail}`);
    }

    const completion = ChatCompletion.parse(await response.json());
    const choice = completion.choices[0];
    const text = choice?.message.content;
    if (text === undefined || text === null) {
      throw new Error("the local model returned no content");
    }
    const inputTokens = completion.usage?.prompt_tokens ?? 0;
    const outputTokens = completion.usage?.completion_tokens ?? 0;
    const usage = {
      inputTokens,
      outputTokens,
      totalTokens: completion.usage?.total_tokens ?? inputTokens + outputTokens,
    };

    if (format === undefined) {
      return { role: "assistant", content: [{ type: "text", text }], outputFormat: "text", usage };
    }
    return {
      role: "assistant",
      content: [{ type: "text", text }],
      outputFormat: "json_schema",
      structuredContent: parseJsonValue(text),
      usage,
    };
  };
  return { generate };
}

const HostedModelName = z.custom<ModelName>(
  (value) => typeof value === "string" && /^(openai|anthropic|google|groq|cerebras)\/.+/.test(value),
);

function chooseModel(name: string): { model: ModelOption; local: boolean } {
  if (name.startsWith(LOCAL_PREFIX)) {
    const baseUrl = process.env.LOCAL_MODEL_URL ?? LOCAL_URL_DEFAULT;
    return { model: localModel(name.slice(LOCAL_PREFIX.length), baseUrl), local: true };
  }
  const hosted = HostedModelName.safeParse(name);
  if (!hosted.success) {
    throw new Error(
      `unknown model "${name}": use local/<model> for a model served locally, or <provider>/<model> ` +
        "with provider openai, anthropic, google, groq or cerebras",
    );
  }
  const apiKey = process.env.MODEL_API_KEY;
  return {
    model: { modelName: hosted.data, ...(apiKey ? { apiKey } : {}) },
    local: false,
  };
}

function priceFromEnv(name: string): number | undefined {
  const raw = process.env[name];
  if (raw === undefined || raw.trim() === "") {
    return undefined;
  }
  const value = Number(raw);
  return Number.isFinite(value) && value >= 0 ? value : undefined;
}

function costText(local: boolean, inputTokens: number, outputTokens: number): string {
  if (local) {
    return "0 (local model, not billed)";
  }
  const inPrice = priceFromEnv("PRICE_IN_PER_MTOK");
  const outPrice = priceFromEnv("PRICE_OUT_PER_MTOK");
  if (inPrice === undefined || outPrice === undefined) {
    return "unknown (set PRICE_IN_PER_MTOK and PRICE_OUT_PER_MTOK, USD per million tokens)";
  }
  const cost = (inputTokens * inPrice + outputTokens * outPrice) / 1_000_000;
  return `$${cost.toFixed(6)} USD`;
}

async function main(): Promise<void> {
  const name = process.argv[2] ?? process.env.MODEL ?? DEFAULT_MODEL;
  const { model, local } = chooseModel(name);

  const browser = await localBrowser.launch({ headless: true });
  const stagehand = await Stagehand.create({ browser, model });
  try {
    const page = await stagehand.browser.context.newPage();
    await page.goto(BOOK_URL);

    const result = await stagehand.extract(INSTRUCTION, RawBookSchema, { page });
    const checked = checkBook(result.data);

    if (checked.ok) {
      console.log(JSON.stringify(checked.book, null, 2));
    } else {
      for (const problem of checked.problems) {
        console.error(problem);
      }
      process.exitCode = 1;
    }

    const metrics = await stagehand.metrics();
    console.log(`model: ${name}`);
    console.log(
      `tokens: ${metrics.totalPromptTokens} input, ${metrics.totalCompletionTokens} output, ` +
        `${metrics.totalReasoningTokens} reasoning`,
    );
    console.log(`cost: ${costText(local, metrics.totalPromptTokens, metrics.totalCompletionTokens)}`);
  } finally {
    await stagehand.close();
  }
}

main().catch((error: unknown) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
});
