# Model choice, capabilities, structured output, usage and cost in comparable tools

Facts gathered on 2026-10-15 for [Models, usage and cost in the SDK API](https://github.com/goodfetch/automan/issues/475), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461). Each claim cites the source that owns it. Quotes are under 15 words. Items marked **unverified** could not be checked against a primary source. This note is inspiration for a decision, not a decision.

## 1. Naming a model

| Tool | Form | Source |
|---|---|---|
| Vercel AI SDK (current docs, no version stated) | `model` is a `LanguageModel`: a provider instance (`openai('…')`) or a plain string. "automatically uses the AI Gateway when you pass a model string" in `creator/model-name` form | [generateText reference](https://ai-sdk.dev/docs/reference/ai-sdk-core/generate-text), [AI Gateway provider](https://ai-sdk.dev/providers/ai-sdk-providers/ai-gateway) |
| Vercel AI Gateway | `creator/model-name` IDs; key in `AI_GATEWAY_API_KEY` | [AI Gateway provider](https://ai-sdk.dev/providers/ai-sdk-providers/ai-gateway) |
| OpenRouter | `author/slug` model IDs | [Models](https://openrouter.ai/docs/guides/overview/models) |
| LiteLLM | `provider/model` strings, the first segment naming the provider, plus `api_base` | [LiteLLM docs](https://docs.litellm.ai/docs/completion/json_mode) and the earlier [consumer API research](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/ai-browser-automation.md) (Notte's LiteLLM strings) |
| Pydantic AI | `provider:model` strings with a colon (`'openai:gpt-5.2'`, `'anthropic:claude-sonnet-4-6'`) | [Agents](https://pydantic.dev/docs/ai/core-concepts/agent/) |
| Stagehand v4 | a provider-prefixed `modelName`; a `ModelConfig` object `{ modelName, apiKey?, headers? }`; or a client-side generation callback (`ClientLLM` in TypeScript), at `create()` only. No base URL option. A bare string is documented for Python's `create()` only | [Stagehand v4 reference](https://docs.stagehand.dev/v4/reference/stagehand) |
| Browser Use | its own wrapper classes per provider (`ChatOpenAI`, `ChatAnthropic`, `ChatOllama`, …) and a `BaseChatModel` protocol | [AI research §2.2 item 8](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/ai-browser-automation.md) |

**Local servers' OpenAI-compatible addresses:** Ollama `http://localhost:11434/v1/` ([openai-compatibility.mdx](https://github.com/ollama/ollama/blob/main/docs/api/openai-compatibility.mdx)); llama.cpp server port 8080 ([server README](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md)). LM Studio (`localhost:1234`) and vLLM (`localhost:8000`) defaults: **unverified** here.

## 2. Declaring model capabilities

- **LiteLLM** keeps a community-maintained `model_prices_and_context_window.json` holding prices and limits per model, with flags read by `supports_response_schema()`; `register_model()` adds or overrides entries. By default it fetches the live map from a URL; `LITELLM_LOCAL_MODEL_COST_MAP=True` uses the bundled copy ([token usage](https://docs.litellm.ai/docs/completion/token_usage), [JSON mode](https://docs.litellm.ai/docs/completion/json_mode)).
- **OpenRouter's** `/api/v1/models` returns per model `pricing` (`prompt`, `completion`, `request`, `image`, `internal_reasoning`, `input_cache_read`, `input_cache_write`), `context_length`, `architecture.input_modalities`, `top_provider.max_completion_tokens` and `supported_parameters` (which lists `tools`, `response_format`, `structured_outputs`) ([Models](https://openrouter.ai/docs/guides/overview/models)).
- **Vercel AI Gateway:** `gateway.getAvailableModels()` returns `id`, `name`, `description` and optional `pricing` (`input`, `output`, `cachedInputTokens`, `cacheCreationInputTokens`) ([AI Gateway provider](https://ai-sdk.dev/providers/ai-sdk-providers/ai-gateway)).
- **Ollama's** `/api/show` returns `capabilities`, a "List of supported features", such as `completion`, `thinking` and `vision` ([openapi.yaml](https://github.com/ollama/ollama/blob/main/docs/openapi.yaml)).
- **The AI SDK docs** show a capability table with Image Input, Object Generation, Tool Usage and Tool Streaming columns, "not exhaustive" ([providers and models](https://ai-sdk.dev/docs/foundations/providers-and-models)).
- **Midscene** requires a model family string; **Browser Use** sniffs model names ([AI research §2.2 and §2.3](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/ai-browser-automation.md)).

## 3. Structured output

- **OpenAI:** Structured Outputs with `response_format: { type: "json_schema", strict: true }`; strict mode requires every property listed in `required` and `additionalProperties: false` (well known; the API reference returned 403 to the fetcher, so the constraint list is **unverified** here).
- **Anthropic:** native structured outputs are GA: `output_config.format` with `type: "json_schema"`; the older `output_format` "is deprecated and will be removed". Not supported: recursive schemas, numerical constraints (`minimum`, `maximum`), string length constraints, `additionalProperties` other than `false`. Strict tool use (`strict: true`) is a second route ([structured outputs](https://platform.claude.com/docs/en/build-with-claude/structured-outputs)).
- **Gemini:** `responseMimeType: "application/json"` with `responseSchema` ([generateContent reference](https://ai.google.dev/api/generate-content)); `responseJsonSchema` is **unverified** on that page.
- **llama.cpp server:** `response_format` takes `{"type": "json_schema", "schema": …}` (the schema directly, unlike OpenAI's nesting), enforced by grammar-based sampling ([server README](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md)). **Ollama** lists `response_format` as supported on `/v1/chat/completions` ([openai-compatibility.mdx](https://github.com/ollama/ollama/blob/main/docs/api/openai-compatibility.mdx)).
- **LiteLLM:** "Not all models support passing the json_schema to them natively"; `enable_json_schema_validation=True` validates client-side ([JSON mode](https://docs.litellm.ai/docs/completion/json_mode)).
- **AI SDK:** `NoObjectGeneratedError` carries `text`, `response`, `usage`, `finishReason` and `cause`: a failed structured call still reports its usage ([error reference](https://ai-sdk.dev/docs/reference/ai-sdk-errors/ai-no-object-generated-error)).

Lesson: each format and each local server accepts a different JSON Schema subset and spelling, so a strategy that works for one model is rejected by another; a ladder (native, tool call, prompt and repair) chosen per model, with validation always against the caller's own schema, is what LiteLLM approximates with client-side validation.

## 4. Usage objects

| Source | Input | Cache | Output | Reasoning | Cost |
|---|---|---|---|---|---|
| OpenAI Responses (`ResponseUsage`) | `input_tokens` | `input_tokens_details.cached_tokens`, `.cache_write_tokens` | `output_tokens` | `output_tokens_details.reasoning_tokens` | none |
| OpenAI Chat Completions (`CompletionUsage`) | `prompt_tokens` | `prompt_tokens_details.cached_tokens`, `.cache_write_tokens` | `completion_tokens` | `completion_tokens_details.reasoning_tokens` | none |
| Anthropic Messages | `input_tokens`, **excluding** cached tokens | `cache_creation_input_tokens`, `cache_read_input_tokens` | `output_tokens` | none | none |
| Gemini `usageMetadata` | `promptTokenCount` | `cachedContentTokenCount` | `candidatesTokenCount` | `thoughtsTokenCount` | none |
| OpenRouter | `prompt_tokens` | `prompt_tokens_details.cached_tokens`, `.cache_write_tokens` | `completion_tokens` | `completion_tokens_details.reasoning_tokens` | `cost`, `cost_details.upstream_inference_cost` |
| AI SDK `usage` | `inputTokens` | `inputTokenDetails.{noCacheTokens, cacheReadTokens, cacheWriteTokens}` | `outputTokens` | `outputTokenDetails.reasoningTokens` | none; `raw` keeps the provider's object |
| OpenTelemetry GenAI | `gen_ai.usage.input_tokens`, which "SHOULD include all types of input tokens, including cached" | `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_write.input_tokens` | `gen_ai.usage.output_tokens` | `gen_ai.usage.reasoning.output_tokens` | none |
| Stagehand v4 `metadata.usage` | `inputTokens` | `cachedInputTokens` | `outputTokens` | `reasoningTokens` | none; `inferenceTimeMs` |
| Pydantic AI `RunUsage` | `input_tokens` | (details) | `output_tokens` | (details) | `cost`, a `Decimal` USD estimate |

Sources: openai-python's [`response_usage.py`](https://github.com/openai/openai-python/blob/main/src/openai/types/responses/response_usage.py) and [`completion_usage.py`](https://github.com/openai/openai-python/blob/main/src/openai/types/completion_usage.py); Anthropic [prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching), whose `input_tokens` covers tokens "which were not read from or used to create a cache"; Gemini [generateContent reference](https://ai.google.dev/api/generate-content); OpenRouter [usage accounting](https://openrouter.ai/docs/use-cases/usage-accounting); AI SDK [generateText reference](https://ai-sdk.dev/docs/reference/ai-sdk-core/generate-text), where `usage` is aggregated across steps and `totalUsage` is deprecated; OpenTelemetry [GenAI spans](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/gen-ai/gen-ai-spans.md) and [attribute registry](https://github.com/open-telemetry/semantic-conventions-genai/blob/main/docs/registry/attributes/gen-ai.md), status Development; Stagehand [v4 reference](https://docs.stagehand.dev/v4/reference/stagehand); Pydantic AI [agents](https://pydantic.dev/docs/ai/core-concepts/agent/).

Lesson: the providers disagree on whether input counts include cached tokens (Anthropic's exclude them). OpenTelemetry and the AI SDK converge on total input tokens with cache reads and writes as subsets, and reasoning as a subset of output.

## 5. Cost

- **OpenRouter** returns `usage.cost`, the amount charged, in every response: "Full usage details are now always included automatically"; `usage: { include: true }` is deprecated ([usage accounting](https://openrouter.ai/docs/use-cases/usage-accounting)).
- **Vercel AI Gateway** responses expose `providerMetadata.gateway.generationId`; the cost comes from a second call, `gateway.getGenerationInfo({ id })`, which returns `totalCost` in USD ([AI Gateway provider](https://ai-sdk.dev/providers/ai-sdk-providers/ai-gateway)).
- **LiteLLM** returns `response_cost` on every call, computed from its price map, which it fetches from a URL by default ([token usage](https://docs.litellm.ai/docs/completion/token_usage)). **Browser Use** prices with `calculate_cost=True` from a pricing URL fetched at runtime ([AI research §2.2 item 8](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/ai-browser-automation.md)).
- **Pydantic AI's** `RunUsage.cost` is a best-effort USD estimate from genai-prices; models without pricing data don't contribute to it, so a total can silently leave a model out ([agents](https://pydantic.dev/docs/ai/core-concepts/agent/)).
- **Stagehand v4** reports tokens, never money; `metrics()` is cumulative per instance, and "Calls that throw before returning a result are not recorded" ([v4 reference](https://docs.stagehand.dev/v4/reference/stagehand)).
- **Anthropic** prices cache writes at 1.25× (5 minutes) or 2× (1 hour) the base input price and cache reads at 0.1× on most models (0.05× or 0.025× on some), so tokens alone do not give a price ([prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)).

## 6. Caps on spend

- **Pydantic AI** `UsageLimits` (`request_limit`, `output_tokens_limit`, `tool_calls_limit`, `cost_limit`, …) raise `UsageLimitExceeded`, which ends the run: "This run will error after 3 requests" ([agents](https://pydantic.dev/docs/ai/core-concepts/agent/)).
- **LiteLLM's proxy** rejects a key over its budget before any provider is contacted, with `budget_exceeded`; the caller's program goes on with its other work ([error reference](https://docs.litellm.ai/docs/proxy/error_reference)). Its SDK-wide `litellm.max_budget` raises `BudgetExceededError` (official page not reached; **unverified**).
- **Stagehand** has no cap ([AI research §2.1](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/ai-browser-automation.md)).

## 7. Timeouts and retries

- **openai-python:** "By default requests time out after 10 minutes"; connection errors, 408, 409, 429 and 5xx are "retried 2 times by default" ([README](https://github.com/openai/openai-python/blob/main/README.md)).
- **AI SDK:** no default timeout documented; `maxRetries` defaults to 2 ([generateText reference](https://ai-sdk.dev/docs/reference/ai-sdk-core/generate-text)).
- Anthropic's SDK defaults: **unverified** here.
