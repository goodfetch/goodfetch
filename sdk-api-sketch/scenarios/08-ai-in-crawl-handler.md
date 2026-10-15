# 08. Extract with AI inside a crawl, learning once per page type

## Task statement

> Crawl the first three categories in the side list of https://books.toscrape.com/ and every book they list, following their "next" pages. Extract each book's title, price, number in stock and star rating with a language model instead of hand-written selectors. The model is asked once per page type: later books of the same layout are extracted from what was learned, with no model call, unless a page no longer matches what was learned. Stop all model use once the crawl has spent a set amount (for example 0.50 USD), let the crawl finish without the model, and report which books were left unextracted and why. Report the tokens and cost spent.

## Target

`books.toscrape.com`: category pages and book pages, which share one layout per page type (checked 2026-10-14).

## Leader

Crawlee 3.18.2 (TypeScript): `StagehandCrawler` from `@crawlee/stagehand` 3.18.2, on Stagehand 3.7.3. Crawlee for Python 1.11.0 (released 2026-10-15): the experimental `PydanticAiCrawler` with `PydanticAiSelectorExtractor`, which learns selectors once and replays them, on pydantic-ai-slim 2.56.0.

## Pains

P20 AI on a crawler's page, P03 replay without the model, P42 budgets and cost, P19 structured output; keeps K02 generate once, then own the deterministic artifact.

## Failure points

1. What was learned no longer matches a page with a different layout.
2. The budget runs out part-way: the rest of the crawl must be explicit about what it skipped.
3. The model returns invalid data for one book.
4. The model provider is unreachable.
5. Two handlers learn the same page type at the same time.
6. Cost or usage is not reported.

## Setup

Leaders' programs are type-checked, never run; the model key is the only secret each version needs. Goodfetch's executables come with each package and pinned browsers are fetched on first use (ADRs 0024, 0057).

| Program | Commands | Count |
|---|---|---|
| Crawlee with Stagehand (TypeScript) | `npm install crawlee@3.18.2 @crawlee/stagehand@3.18.2 @browserbasehq/stagehand@3.7.3 playwright@1.64.1 zod@4.6.7`; `npx playwright install chrome` (Stagehand's `LOCAL` env launches an installed Chrome, which Crawlee then drives over CDP); `export OPENAI_API_KEY=…` | 3 |
| Crawlee for Python with PydanticAI | `pip install "crawlee[pydantic-ai]==1.11.0"` (an HTTP crawler: no browser); `export OPENAI_API_KEY=…` | 2 |
| Goodfetch TypeScript | `npm install @goodfetch/foxhound zod`; `export AI_GATEWAY_API_KEY=…` (the Vercel AI Gateway, ADR 0076) | 2 |
| Goodfetch Python | `pip install goodfetch-foxhound pydantic`; `export AI_GATEWAY_API_KEY=…` | 2 |
| Goodfetch Rust | `cargo add goodfetch-foxhound serde -F serde/derive`; `cargo add tokio -F macros,rt-multi-thread`; `export AI_GATEWAY_API_KEY=…` | 3 |

The leaders use OpenAI directly, their documented default (`openai/gpt-…` in Stagehand, `openai:…` in Pydantic AI). Pydantic AI also reaches the Vercel AI Gateway (`vercel:openai/gpt-5-mini`), but genai-prices 0.1.13 has no `vercel` provider, so its price could not be computed there.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Crawlee with Stagehand (TypeScript) | 34 | 102 | 3 | 0 / 3 / 3 |
| Crawlee for Python with PydanticAI | 32 | 98 | 2 | 1 / 4 / 1 |
| Goodfetch TypeScript | 44 | 51 | 2 | 6 / 0 / 0 |
| Goodfetch Python | 46 | 66 | 2 | 6 / 0 / 0 |
| Goodfetch Rust | 54 | 125 | 3 | 6 / 0 / 0 |

Programs: `scenarios/leaders/stagehand3/08-ai-in-crawl-handler.crawlee-stagehand.ts`, `scenarios/leaders/s08_ai_in_crawl_handler_crawlee.py`, `typescript/scenarios/08-ai-in-crawl-handler.ts`, `python/scenarios/s08_ai_in_crawl_handler.py`, `rust/examples/s08_ai_in_crawl_handler.rs`. Lines are counted by the formatters' defaults (oxfmt, ruff format, rustfmt). The Goodfetch programs keep the crawl's declaration apart from `main` (module-level values, `import.meta.main`, `__main__`, `pub fn crawl()`) so scenario 10's offline tests reuse it; Rust's `pub` struct of handles costs about 15 of its lines.

**What each leader needs code for.** Stagehand 3.7.3 caches `act` and the agent (`cacheDir`: `ActCache`, `AgentCache`) and never `extract`, which asks the model on every page, so learning once per page type is the program's own: one `observe` per field, the XPaths kept per label, each later book read with Playwright, and the text parsed by Zod transforms. `PydanticAiSelectorExtractor` learns and replays selectors itself, bucketed per request label, but copies text, so the schema parses the price, stock and rating with validators. Neither caps spend across a crawl (pydantic-ai's `usage_limits` bounds one generation run): the Stagehand program sums each instance's token metrics and prices them from a hand-written table, and the Python program wraps the model in a `WrapperModel` that prices each response with genai-prices and refuses requests past the budget. Goodfetch's `extract` compiles each field's locators and transform into the Plan once per layout (ADR 0091), and the Model allowance (ADR 0107) caps the crawl's model use while the crawl goes on.

### Concepts counted

- **Crawlee with Stagehand (34):** `StagehandCrawler`, `createStagehandRouter`, `Dataset`, `pushData`, `Stagehand`, `StagehandPage`, `stagehandOptions`, `env`, `"LOCAL"`, `model`, `requestHandler`, `failedRequestHandler`, `addDefaultHandler`, `addHandler`, `enqueueLinks`, `selector`, `label`, `limit`, `page`, `request`, `url`, `loadedUrl`, `errorMessages`, `stagehand`, `observe`, `metrics`, `totalPromptTokens`, `totalCompletionTokens`, `locator`, `textContent`, `timeout`, `getAttribute`, `run`; must await the `metrics` getter.
- **Crawlee for Python with PydanticAI (32):** `BasicCrawlingContext`, `PydanticAiCrawler`, `PydanticAiCrawlingContext`, `PydanticAiSelectorExtractor`, `PydanticAiDirectExtractor`, `fallback`, `extractor`, `ModelMessage`, `ModelResponse`, `ModelRequestParameters`, `KnownModelName`, `ModelSettings`, `WrapperModel`, `request` (the model's, overridden), `cost()`, `total_price`, the `provider:model` name form, `router.default_handler`, `router.handler`, `failed_request_handler`, `enqueue_links`, `selector`, `label`, `limit`, `extract`, `push_data`, `request` (the crawl's), `url`, `run`, `ai_usage`, `total_tokens`, `requests`.
- **Goodfetch TypeScript (44):** `Crawl`, `Foxhound`, `seed`, `dataset`, `schema`, `key`, `label`, `settings`, `timeout.handler`, `handle`, `page`, `enqueueLinks`, `limit`, `locator`, `extract`, `output`, `push`, `url`, `connect`, `ai.model`, `startCrawl`, `seeds`, `plan`, `ai.allowance.price`, `Money` (`amount`, `currency`), `outcomes`, `kinds`, `labels`, `kind`, `"failed"`, `"skipped"`, `request`, `reason`, `message`, `error`, `code`, `fix`, `finish`, `usage`, `model` (the report's), `tokens`, `price`; must dispose the connection and the Run (`await using`).
- **Goodfetch Python (46):** the TypeScript list with `AsyncCrawl`, `AsyncFoxhound`, `enqueue_links`, `start_crawl` for their twins, and `AsyncCrawlContext`, `CrawlRequestSkipped`, `CrawlRequestFailed` in place of `kind`; must close the connection and the Run (`async with`).
- **Goodfetch Rust (54):** `ConnectOptions`, `Crawl`, `CrawlContext`, `CrawlRequestOutcomeKind`, `Skipped`, `Failed`, `Dataset`, `DatasetDeclaration`, `key`, `Error`, `Foxhound`, `Label`, `LabelDeclaration`, `Money`, `OutcomeKind`, `OutcomesFilter`, `kinds`, `label`, `Schema::of`, `Seed`, `Settings`, `timeout::HANDLER`, `ai::MODEL`, `ai::allowance::PRICE`, `dataset`, `handle`, `page`, `enqueue_links`, `limit`, `locator`, `request`, `extract`, `output`, `push`, `url`, `connect`, `start_crawl`, `plan`, `serve`, `outcomes`, `next`, `reason`, `message`, `fix`, `code`, `run` (the report's), `usage`, `model`, `tokens`, `price`, `amount`/`currency`, `as_run`; must close the Run and the connection.

### Failure points, scored

| Failure point | Crawlee with Stagehand (TS) | Crawlee for Python with PydanticAI | Goodfetch (TS, Python, Rust) |
|---|---|---|---|
| 1. What was learned no longer matches a page with a different layout | silent: a read that times out or fails Zod observes again, but a kept absolute XPath that lands on another parsable element returns wrong data | silent: a cached selector map that still validates on the new layout returns its values; only a map that fails to apply is regenerated, as a new variant (at most 5) | typed: each replay checks the step's Target fingerprints; on Drift one Repair (`step.source` `repair`, with the Drift), else `PlanDriftError`, the request ending `failed` with `plan_drift` |
| 2. The budget runs out part-way | generic: no cap in the library; the program's check of its own token sum, which concurrent handlers pass together and overshoot, and a reason it writes as text | generic: no crawl-wide cap; the program's `BudgetReachedError` from its `WrapperModel`, check-then-charge, so concurrent generations overshoot; learned selectors keep extracting | typed: `ai.allowance.price` reserves each call's worst case, so it is never passed; `ModelAllowanceReachedError`, the request `skipped` with the reason `model_allowance`, the Run `incomplete` though `finished` |
| 3. The model returns invalid data for one book | generic: the program's `ZodError` after a second observe; Crawlee retries the request three times, observing again each time, then `errorMessages` as text | generic: `ModelRetry` up to 3 times, then the fallback reads the page with the model; its `UnexpectedModelBehavior` is retried three times by Crawlee, then read as text | typed: `ExtractionInvalidError` (`extraction_invalid`) with the model, the strategy, each field's issue and what the attempts spent; the request ends `failed` with it |
| 4. The model provider is unreachable | generic: Crawlee rethrows `Error("Stagehand observe() failed: …")`, retried, then text | generic: the provider SDK's connection error or pydantic-ai's `ModelHTTPError`, passed through by the extractor, retried, then text | typed: `ModelUnavailableError` (`model_unavailable`, reason `unreachable`, retryable, `retryAfter`), retried under `crawl.retry.max`, then the request's `failed` outcome |
| 5. Two handlers learn the same page type at the same time | silent: both observe, paying twice, and the last write wins | typed (prevented): one `asyncio.Lock` per schema, scope and label; the second waits and replays the first's selectors, within one process | typed (prevented): within a Run one entry and layout compiles at most once at a time; the second call waits and replays it (`step.source` `plan`, no model call), ADR 0091 |
| 6. Cost or usage is not reported | silent: tokens per Stagehand instance (one per browser), summed by the program; no price, so its hand-written table goes stale unnoticed | generic: tokens on `crawler.ai_usage`; a price only through genai-prices in the program's wrapper, `LookupError` for a model it does not price | typed: `report.usage.model` carries calls, tokens and `price` from the price table, `null` for an unpriced model, which a price allowance refuses with `model_price_unknown` |

A failure point the library prevents is scored typed: nothing reaches the program, and the behavior is documented.

### Verdict

- **Lines:** Goodfetch TypeScript and Python are about half the leaders' (51 and 66 against 102 and 98), because learning once, the transforms, the cap and the price are the engine's; Rust's 125 carry the `pub` handle struct scenario 10 includes and rustfmt's layout, and have no scored leader.
- **Concepts:** Goodfetch's programs name more (44 and 46 against 34 and 32), because each reads typed outcomes, reasons and fixes where the leaders keep a list of strings of their own; the leaders' own budget, price table, parsers and selector cache are code a reader must also hold, and they are not counted. The cost is G1 and G6: what was left unextracted, and why, comes from the Run itself.
- **Setup:** equal to the Python leader's and one command fewer than Stagehand's, which needs an installed Chrome.
- **Failure points:** every Goodfetch failure point is typed, where the leaders are silent on a drifted layout that still parses (G1) and on usage (Stagehand, G6), and generic on the budget, the invalid answer and the outage.

## After the sketch

A new Journey on the SDK surfaces; Proving ground families: AI, extraction, and the crawl families, served by the Fake model.
