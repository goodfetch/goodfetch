// Snippet for Models, usage and cost in the SDK API (#475, ADR 0101), on golden scenarios 02, 08 and 11.
import { z } from "zod";
import {
  Goodfetch,
  ExtractionIncompleteError,
  ExtractionInvalidError,
  ModelUnavailableError,
  type GoodfetchError,
  type Logger,
  type ModelProviderHandler,
  type TokenCounts,
  type Page,
  type Run,
} from "goodfetch";

declare const log: Logger; // the program's own logger

const Book = z.object({
  title: z.string(),
  price: z.string().regex(/^\d+\.\d{2}$/), // pounds, exact
  inStock: z.number().int(),
  rating: z.number().int().min(1).max(5),
  description: z.string(),
});

const usageFields = (usage: TokenCounts) => ({
  calls: usage.calls,
  tokens: usage.tokens,
  cachedInput: usage.cacheReadTokens,
  price: usage.price === null ? "unpriced" : `${usage.price.amount} ${usage.price.currency}`,
});

// Golden scenario 02, "Extract a typed object with a chosen model". The model is a Model reference:
// a provider's name, a slash, and that provider's own model ID. The connection's setting serves every
// call; one call may name its own, here a local model served by Ollama, priced zero at its default address.
export async function typedBook(local: boolean): Promise<z.infer<typeof Book> | undefined> {
  await using gf = await Goodfetch.connect({
    logger: log,
    settings: { ai: { model: "anthropic/claude-sonnet-4-5" } }, // key: env:ANTHROPIC_API_KEY, sent only to Anthropic
  });
  await using run = await gf.collie.start();
  const page = await run.newPage();
  await page.goto("https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html");
  try {
    const { output, usage } = await page.extract("the book on this page", Book, local ? { model: "ollama/qwen3:8b" } : {});
    log.info("extracted", usageFields(usage)); // always there, priced from the price table
    return output;
  } catch (error) {
    if (error instanceof ExtractionIncompleteError) {
      log.warn("fields not on the page", { missing: error.details.missing });
      return undefined;
    }
    if (error instanceof ExtractionInvalidError) {
      // Names the model and the strategy; what the failed attempts spent is on the error too.
      const { model, strategy, issues } = error.details;
      log.error("the model never fitted the schema", { model, strategy, issues, ...(error.usage ? usageFields(error.usage) : {}) });
    }
    throw error;
  }
}

// Model providers: a gateway, a declared OpenAI-compatible server, and the program's own handler.
// Capabilities are declared, never guessed from the name; prices are set only in configuration.
declare function inference(prompt: string, signal: AbortSignal): Promise<{ text: string; inTokens: number; outTokens: number }>;
const myModel: ModelProviderHandler = async (request) => {
  // Goodfetch's own types: no provider SDK. The engine counts, caps and journals the call.
  const prompt = request.messages.map((message) => JSON.stringify(message)).join("\n");
  const answer = await inference(prompt, request.signal); // aborted past timeout.model
  return { text: answer.text, finishReason: "stop", usage: { inputTokens: answer.inTokens, outputTokens: answer.outTokens } };
};
export async function providers(): Promise<void> {
  await using gf = await Goodfetch.connect({
    modelProviders: { inhouse: myModel }, // "inhouse/<anything>" reaches it
    settings: {
      ai: {
        model: "vercel/openai/gpt-5-mini", // the Vercel AI Gateway, env:AI_GATEWAY_API_KEY (ADR 0076)
        structuredOutput: "auto", // the built-in: native, then a forced tool call, then the prompt
        providers: {
          lab: {
            format: "openai",
            baseUrl: "http://gpu-box.internal:8000/v1",
            apiKey: "env:LAB_VLLM_KEY", // a Secret reference, sent only to baseUrl
            models: { "qwen3-vl-32b": { imageInput: true, contextWindow: 131_072, strategies: ["native", "prompt"] } },
          },
        },
      },
      timeout: { model: 120_000, ai: 300_000 }, // a slow server: one model request may take 2 minutes, an AI call 5
    },
  });
  await using run = await gf.collie.start({ settings: { ai: { model: "lab/qwen3-vl-32b" } } });
  const page = await run.newPage();
  await page.goto("https://books.toscrape.com/");
  await page.act("open the Travel category");
  // The program's own model for one call: it gets the prompt strategy, and only configuration prices it.
  await page.extract("the books listed", Book.pick({ title: true }).array(), { model: "inhouse/ranker-v2" });

  // @ts-expect-error a Model reference names its provider: "provider/model"
  await page.act("open the first book", { model: "gpt-5-mini" });
}

// Run totals, what remains of each Limit, and the uncapped mark (ADR 0041). The price Limit and
// `usage.price` are the same `Money`.
export async function cappedRun(gf: Goodfetch): Promise<void> {
  await using run = await gf.collie.start({ limits: { price: { amount: "0.20", currency: "USD" }, modelCalls: 40 } });
  const page = await run.newPage();
  await page.goto("https://books.toscrape.com/");
  await page.act("open the Travel category");

  const usage = await run.usage();
  log.info("so far", { ...usageFields(usage.model), browserTime: usage.browserTime, uncapped: usage.uncapped });
  for (const limit of usage.limits) log.info(`${limit.scope} ${limit.kind}`, { remaining: limit.remaining, setBy: limit.setBy });
}

// A provider outage is retryable and says when to come back; a rejected key or an exhausted credit is not.
export async function withOutage(page: Page): Promise<void> {
  try {
    await page.act("add the first book to the basket");
  } catch (error) {
    if (error instanceof ModelUnavailableError && error.details.reason === "rate_limited") {
      log.warn("rate limited", { retryAfter: error.details.retryAfter, model: error.details.model });
      return;
    }
    const failure = error as GoodfetchError;
    if (failure.code === "model_unauthorized" || failure.code === "model_funds_exhausted") log.error(failure.message);
    throw error;
  }
}

// Golden scenario 08's Usage report. Its "stop model use, finish without the model" is
// "Capping a crawl's model use without stopping its Run" (#513): here the price Limit stops the crawl.
export async function reportSpend(run: Run): Promise<void> {
  const { model, price, uncapped } = await run.usage();
  for (const used of model.byModel) log.info(used.model, usageFields(used));
  log.info("spent", { price, uncapped });
}
