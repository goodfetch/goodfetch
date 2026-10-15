// Models, usage and cost in the SDK API (ADR 0101, decided in #475).
// - A program names a model by a Model reference, `"<provider>/<model id>"`: the setting `ai.model`
//   for a scope, or the AI verbs' per-call `model` (ADR 0091), which is that setting at the run layer.
//   `ai.model` has no default: no model is called unless one is configured (ADR 0043).
// - Model providers are built in (`openai`, `anthropic`, `google`, `vercel`, `openrouter`, and the
//   local `ollama`, `llamacpp`, `lmstudio`, `vllm`), declared under `ai.providers.<name>`, or an SDK
//   handler registered on the connection (ADR 0033's SDK-handler form of the Model provider kind).
// - Capabilities are declared, never sniffed from a name; prices come from the price table, set only
//   in configuration, so no Client can lower the price a person's currency Limit is held to.
// - Every AI result carries its `ModelUsage`, priced; `run.usage()` gives the Run's totals, what
//   remains of each Limit and ADR 0041's uncapped mark. An error from a call that made model calls
//   carries its `usage` too.

import type { JsonSchema } from "./data.js";
import type { ModelError, ProgramError, RefusedError, UnavailableError } from "./results.js";
import type { HeadersInput, SecretReference } from "./sessions.js";
import type { CallOptions, Money } from "./time.js";

// Naming a model.

/**
 * A Model reference: a Model provider's name, a slash, and that provider's own model ID verbatim, so a
 * gateway's ID keeps its slash: `"anthropic/claude-sonnet-4-5"`, `"vercel/anthropic/claude-sonnet-4.5"`,
 * `"ollama/qwen3:8b"`. The provider must be built in or declared where the scope opens
 * (`setting_invalid`, naming the closest); the model ID is the provider's to accept (`model_not_found`).
 */
export type ModelReference = `${string}/${string}`;

/** The built-in Model providers. A local one sits at its server's default loopback address. */
export type BuiltinModelProvider =
  | "openai"
  | "anthropic"
  | "google"
  | "vercel"
  | "openrouter"
  | "ollama"
  | "llamacpp"
  | "lmstudio"
  | "vllm";

/** ADR 0012's three request formats. */
export type ModelFormat = "openai" | "anthropic" | "gemini";

/** How the engine gets JSON that fits a schema: the provider's schema mode, a forced tool call, or the schema in the prompt with repair. */
export type StructuredOutputStrategy = "native" | "tool" | "prompt";

/**
 * What a model can take and produce, declared rather than inferred from its name. The shipped list
 * covers the tested models (ADR 0065); a model it does not know is text only, gets structured output
 * from the top of the ladder (`prompt` for the program's own provider), and has no price. Its price
 * is the price table's, overridden only in configuration under
 * `ai.providers.<name>.models.<id>.price`, so it is no key here: a price set in code fails with
 * `setting_locked`. A model ID holding a dot or a slash is one quoted segment of a key:
 * `ai.providers.lab.models."gpt-5.1".image_input`.
 */
export interface ModelDeclaration {
  readonly contextWindow?: number;
  readonly maxOutputTokens?: number;
  readonly imageInput?: boolean;
  readonly toolCalls?: boolean;
  /** The structured output strategies the model accepts, best first. */
  readonly strategies?: readonly StructuredOutputStrategy[];
}

/**
 * `ai.providers.<name>`: a Model provider the program declares, or a built-in it changes (a proxy in
 * front of OpenAI), at the connection, a Run or a Session. `apiKey` and secret `headers` go only to
 * `baseUrl`. A built-in's default key (its conventional variable, such as `env:OPENAI_API_KEY`) goes
 * only to its own address, so a built-in pointed elsewhere sends none. `apiKey`, `headers` and
 * `baseUrl` are set by the program's own code, configuration or an Operator policy: a Recipe and a
 * Client acting for a model, an Integration included, never set them (`setting_locked`). For the
 * program's own provider, only `models` applies.
 */
export interface ModelProviderSettings {
  readonly format?: ModelFormat;
  readonly baseUrl?: string;
  readonly apiKey?: SecretReference | null;
  readonly headers?: HeadersInput;
  /** Capabilities by the provider's own model ID. */
  readonly models?: Readonly<Record<string, ModelDeclaration>>;
}

/** `ai.*`. */
export interface AiSettings {
  /** `ai.model`: the model every AI call of the scope uses unless the call names its own. No default. */
  readonly model?: ModelReference | null;
  readonly providers?: Readonly<Record<string, ModelProviderSettings>>;
  /**
   * `ai.structured_output`: `auto` (the default) starts at the best strategy the model declares, or
   * `native` for one it does not know, and steps down when the provider rejects a strategy or a schema
   * feature; an answer that fails validation is retried, never stepped down. Every answer is validated
   * against the program's own schema and validators (ADR 0082).
   */
  readonly structuredOutput?: "auto" | StructuredOutputStrategy;
}

// The SDK handler: ADR 0033's SDK-handler form of the Model provider kind.

export type ModelMessage =
  | { readonly role: "system"; readonly content: string }
  | { readonly role: "user"; readonly content: readonly ModelContentPart[] }
  | { readonly role: "assistant"; readonly content: readonly ModelContentPart[]; readonly toolCalls?: readonly ModelToolCall[] }
  | { readonly role: "tool"; readonly toolCallId: string; readonly content: string };

export type ModelContentPart =
  | { readonly type: "text"; readonly text: string }
  | { readonly type: "image"; readonly mediaType: string; readonly data: Uint8Array };

export interface ModelTool {
  readonly name: string;
  readonly description: string;
  readonly inputSchema: JsonSchema;
}

/** Whether the answer must call a tool: `tool` is the forced tool call of the `tool` strategy. */
export type ModelToolChoice = { readonly type: "auto" } | { readonly type: "none" } | { readonly type: "tool"; readonly name: string };

export interface ModelToolCall {
  readonly id: string;
  readonly name: string;
  /** The arguments as JSON text, as the model wrote them: the engine parses and validates them. */
  readonly arguments: string;
}

/** What the engine asks a handler: Goodfetch's own types, exported, so no provider SDK is needed (G7). */
export interface ModelRequest {
  /** The model ID after the provider's name in the Model reference. */
  readonly model: string;
  readonly messages: readonly ModelMessage[];
  readonly tools: readonly ModelTool[];
  readonly toolChoice: ModelToolChoice;
  /** Set when the strategy is `native`: the answer must be JSON fitting it. */
  readonly responseSchema: JsonSchema | null;
  readonly maxOutputTokens: number;
  /** Aborted when `timeout.model` passes or the call is cancelled (`$/cancel`). */
  readonly signal: AbortSignal;
}

/** The token counts a handler reports: input and output are required, so a Limit counts every call. */
export interface ReportedTokens {
  readonly inputTokens: number;
  readonly outputTokens: number;
  readonly cacheReadTokens?: number;
  readonly cacheWriteTokens?: number;
  readonly reasoningTokens?: number;
}

/** The program's answer; a refusal is `finishReason: "refusal"`, which the engine raises as `model_declined`. */
export interface ModelResponse {
  readonly text: string;
  readonly toolCalls?: readonly ModelToolCall[];
  readonly finishReason: "stop" | "length" | "tool_calls" | "refusal" | (string & {});
  readonly usage: ReportedTokens;
}

/**
 * Answers model calls for the provider name it is registered under, where the program's own code runs
 * (ADR 0081). The engine journals, counts and caps each call (ADR 0012); it gets the `prompt` strategy
 * unless its models declare others, and only configuration prices it. A `GoodfetchError` it throws
 * keeps its code (a `ModelUnavailableError` is retried as a provider's would be); any other exception
 * is `model_handler_failed`.
 */
export type ModelProviderHandler = (request: ModelRequest) => Promise<ModelResponse>;

// Usage and its price.

/**
 * Token counts in OpenTelemetry's and the AI SDK's convention, whatever the provider's: input counts
 * every input token, cached ones included (Anthropic's three counts are summed), and output counts
 * reasoning tokens.
 */
export interface TokenCounts {
  /** Model calls sent, retries and Repairs included; a step replayed from the Plan sends none. */
  readonly calls: number;
  /** Input plus output: what `limits.modelTokens` counts. */
  readonly tokens: number;
  readonly inputTokens: number;
  /** Part of `inputTokens`. */
  readonly cacheReadTokens: number;
  /** Part of `inputTokens`. */
  readonly cacheWriteTokens: number;
  readonly outputTokens: number;
  /** Part of `outputTokens`. */
  readonly reasoningTokens: number;
  /**
   * Their price from the price table: a list-price estimate, never a bill, in the table's currency
   * (USD as shipped), other currencies converted only through the table's fixed rates. `null` when
   * part of it has no price, or needs a rate the table lacks: never a partial sum.
   */
  readonly price: Money | null;
}

export interface ModelUsageOf extends TokenCounts {
  readonly model: ModelReference;
}

/** What model calls consumed: on every AI result, and on every error from a call that made model calls. */
export interface ModelUsage extends TokenCounts {
  /** The same per Model reference, in first-use order. */
  readonly byModel: readonly ModelUsageOf[];
}

/** Where a Limit attaches (ADR 0041). A newer engine's value is kept as received (G3). */
export type LimitScope =
  | "run"
  | "connection"
  | "named_token"
  | "person"
  | "daemon"
  | "install"
  | "schedule"
  | "partitioned_crawl"
  | (string & {});

/** What a Limit caps, as `Limits` names it. */
export type LimitKind =
  | "model_tokens"
  | "model_calls"
  | "browser_time"
  | "proxy_bytes"
  | "paid_solves"
  | "paid_fetches"
  | "price"
  | (string & {});

/** A Limit that applies to the Run, and what remains of it. */
export interface AppliedLimit {
  readonly scope: LimitScope;
  readonly kind: LimitKind;
  /** A count, milliseconds for `browser_time`, or `Money` for `price`. */
  readonly value: number | Money;
  readonly remaining: number | Money;
  /** Who set it (ADRs 0041, 0045): only a person's or a managed policy's Limit allows Paid fetches. */
  readonly setBy: "person" | "policy" | "client" | (string & {});
}

/** A Run's Usage so far: the figures the Usage ledger charges. */
export interface RunUsage {
  readonly model: ModelUsage;
  /** Milliseconds of browser time. */
  readonly browserTime: number;
  readonly proxyBytes: number;
  readonly paidSolves: number;
  readonly paidFetches: number;
  /**
   * The price of the Usage third parties charge for: model calls, metered proxies, paid solves and
   * Paid fetches; `null` as `TokenCounts.price` when any of it has no price or rate. Browser time and
   * an unmetered proxy carry none.
   */
  readonly price: Money | null;
  readonly limits: readonly AppliedLimit[];
  /** ADR 0041's mark: the Run can spend money with nothing capping it. */
  readonly uncapped: boolean;
}

/** A Run reads its Usage while it is open; its report carries the final one (The Run in code, #478). */
export interface ReadsUsage {
  usage(options?: CallOptions): Promise<RunUsage>;
}

// The Model provider Error codes (ADR 0079's groups). `model_not_configured` is ADR 0091's (ai.d.ts).

/** What every model failure names. */
export interface ModelFailureDetails {
  readonly model: ModelReference;
  /** The provider's HTTP status, when one answered. */
  readonly status: number | null;
  /** The provider's request ID, when it sent one. */
  readonly requestId: string | null;
}

/** The provider says the model does not exist or is not available to this key. */
export declare class ModelNotFoundError extends ProgramError {
  readonly code: "model_not_found";
  readonly details: ModelFailureDetails;
}

/** The operation needs a capability the model does not declare; `fix` names the key that declares it. */
export declare class ModelCapabilityMissingError extends ProgramError {
  readonly code: "model_capability_missing";
  readonly details: { readonly model: ModelReference; readonly capability: "image_input" | "tool_calls" | (string & {}) };
}

/** The SDK handler threw an exception that is not a `GoodfetchError`; `cause` holds it. */
export declare class ModelHandlerFailedError extends ProgramError {
  readonly code: "model_handler_failed";
  readonly details: { readonly provider: string; readonly exceptionType: string; readonly exceptionMessage: string };
}

/** The provider rejected the key. Not retryable: fix the provider's `apiKey`. */
export declare class ModelUnauthorizedError extends RefusedError {
  readonly code: "model_unauthorized";
  readonly retryable: false;
  readonly details: ModelFailureDetails;
}

/** The provider refused because the account has no funds: no credit left, an unpaid account, a used-up free tier. Not retryable. */
export declare class ModelFundsExhaustedError extends RefusedError {
  readonly code: "model_funds_exhausted";
  readonly retryable: false;
  readonly details: ModelFailureDetails;
}

/**
 * A currency Limit applies and the model's price cannot be stated in its currency: the price table has
 * no price for the model, or no fixed rate to the Limit's currency. `fix` names the configuration key.
 */
export declare class ModelPriceUnknownError extends RefusedError {
  readonly code: "model_price_unknown";
  readonly details: { readonly model: ModelReference; readonly missing: "price" | "rate" | (string & {}) };
}

/** Unreachable, a 5xx, or rate limited after the engine's retries. Retryable. */
export declare class ModelUnavailableError extends UnavailableError {
  readonly code: "model_unavailable";
  readonly retryable: true;
  readonly details: ModelFailureDetails & {
    readonly reason: "unreachable" | "server_error" | "rate_limited" | (string & {});
    /** Milliseconds the provider asked to wait, when it said. */
    readonly retryAfter: number | null;
  };
}

/** The model declined to answer: a refusal or a content filter. */
export declare class ModelDeclinedError extends ModelError {
  readonly code: "model_declined";
  readonly details: ModelFailureDetails & { readonly reason: string | null };
}

/** The request does not fit the model's context window: narrow the scope, or choose a larger model. */
export declare class ModelContextExceededError extends ModelError {
  readonly code: "model_context_exceeded";
  readonly details: ModelFailureDetails & { readonly contextWindow: number | null };
}

/** An `act`, `observe` or Agent step's answer never parsed within ADR 0012's retries. */
export declare class ModelOutputInvalidError extends ModelError {
  readonly code: "model_output_invalid";
  readonly details: {
    readonly model: ModelReference;
    readonly strategy: StructuredOutputStrategy;
    readonly attempts: number;
    /** The last attempt's problem. */
    readonly problem: string;
  };
}
