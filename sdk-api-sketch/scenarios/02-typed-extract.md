# 02. Extract a typed object with a chosen model

## Task statement

> From https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html, extract the book as a typed object: its title (text), its price (a decimal number of pounds), the number of copies in stock (an integer), its star rating (one of one to five) and its description (text). Use a language model to read the page instead of hand-written selectors. The program chooses the model by name, and a model served locally must work as well as a hosted one. Print the tokens used and their cost next to the result. If a field cannot be found or does not have the expected type, say which field, instead of returning an empty or wrong value.

## Target

A product page of `books.toscrape.com`: "A Light in the Attic", £51.77, 22 in stock, three stars (checked 2026-10-14).

## Leader

Stagehand 4.2.1, `extract` with a Zod schema (TypeScript).

## Pains

P19 model breadth and structured output, P04 false success, P42 cost visibility, P03 per-run AI cost; keeps K08 any model, including local ones.

## Failure points

1. The model's answer does not match the declared types (the price as `"£51.77"`, the rating as `"Three"`).
2. A field is absent from the page.
3. The model provider is unreachable, refuses, or rate-limits.
4. The model is not able to give structured output at all.
5. The page did not load, or loaded a different page.
6. Usage or cost is not reported.

## Setup

Counted apart from lines. Each version needs the hosted model's key, and a run with `--local` needs Ollama running with the model pulled. Goodfetch's engine comes with each package and its pinned browser is fetched on first use (ADRs 0024, 0057); Stagehand 4 launches the Chrome installed on the machine, or the one `CHROME_PATH` names, and fails with "Chrome installation not found" otherwise.

| Program | Commands | Count |
|---|---|---|
| Stagehand (TypeScript) | `npm install @browserbasehq/stagehand@4.2.1 zod@4.4.3` (Stagehand 4.2.1 depends on exactly zod 4.4.3, and a schema from another zod release fails `tsc` at `extract`); Google Chrome installed; `export ANTHROPIC_API_KEY=…`; `ollama pull qwen3:8b` | 4 |
| Goodfetch TypeScript | `npm install @goodfetch/collie zod`; `export ANTHROPIC_API_KEY=…`; `ollama pull qwen3:8b` | 3 |
| Goodfetch Python | `pip install goodfetch-collie pydantic`; `export ANTHROPIC_API_KEY=…`; `ollama pull qwen3:8b` | 3 |
| Goodfetch Rust | `cargo add goodfetch-collie serde --features serde/derive`; `cargo add tokio --features macros,rt-multi-thread`; `export ANTHROPIC_API_KEY=…`; `ollama pull qwen3:8b` | 4 |

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Stagehand (TypeScript) | 41 | 78 | 4 | 0 / 4 / 2 |
| Goodfetch TypeScript | 28 | 43 | 3 | 6 / 0 / 0 |
| Goodfetch Python | 28 | 56 | 3 | 6 / 0 / 0 |
| Goodfetch Rust | 36 | 63 | 4 | 6 / 0 / 0 |

Programs: [`leaders/02-typed-extract.stagehand.ts`](leaders/02-typed-extract.stagehand.ts), [`../typescript/scenarios/02-typed-extract.ts`](../typescript/scenarios/02-typed-extract.ts), [`../python/scenarios/s02_typed_extract.py`](../python/scenarios/s02_typed_extract.py), [`../rust/examples/s02_typed_extract.rs`](../rust/examples/s02_typed_extract.rs). Each takes `--local` to use `qwen3:8b` served by Ollama instead of `anthropic/claude-sonnet-4-5`. The price means the same in every Goodfetch program, an exact decimal number of pounds: text matching `^\d+\.\d{2}$` in TypeScript and Rust, which have no decimal type of their own, and a `Decimal` of two places in Python, which Pydantic writes as text in JSON. Stagehand's program keeps a Zod `number`, its README's form. Every program checks the page's title before extracting, Stagehand's with `page.title()`.

What was verified in Stagehand 4.2.1's installed types and its docs, and shapes its program: a model name must start with `openai/`, `anthropic/`, `google/`, `groq/` or `cerebras/` (`ModelConfigSchema`), and there is no base URL, so a model served locally is a `generate` callback the program writes against the server's own API ("Custom models", docs.stagehand.dev/v4/configuration/models), here 47 lines against Ollama's OpenAI-compatible endpoint; `metadata.usage` gives tokens and no price, so the program keeps its own price table; `extract` parses the answer with the program's Zod schema after the call (`resolvedSchema.parse(response.data)`), so a mismatch is a `ZodError` and the call's `metadata` is lost with it.

**Concepts counted** (each name once per program; obligations last):

- Stagehand (TypeScript), 41: `localBrowser`, `launch`, `Stagehand`, `create`, `browser`, `model`, `modelName`, `apiKey`, the `"anthropic/…"` name form, `ClientLLM`, `generate`, `responseFormat`, `"json_schema"`, `name`, `schema`, `systemPrompt`, `messages`, `role`, `"assistant"`, `content`, `type`, `"text"`, `text`, `temperature`, `outputFormat`, `structuredContent`, `usage`, `inputTokens`, `outputTokens`, `totalTokens`, `context`, `pages`, `goto`, `title`, `extract`, `data`, `metadata`, `close`; must close Stagehand, must close the browser, must await. The Ollama endpoint's own fields (`response_format`, `choices`, `prompt_tokens`, …) are not counted, though the reader must know them too.
- Goodfetch TypeScript, 28: `Collie`, `connect`, `start`, `newPage`, `goto`, `expect`, `toHaveTitle`, `extract`, `model`, the `"provider/model"` Model reference form, `output`, `usage`, `tokens`, `inputTokens`, `outputTokens`, `price`, `amount`, `currency`, `ExtractionIncompleteError`, `details`, `missing`, `path`, `ExtractionInvalidError`, `issues`, `message`, `finish`; must close (`await using`), must await.
- Goodfetch Python, 28: `AsyncCollie`, `connect`, `start`, `new_page`, `goto`, `expect`, `to_have_title`, `extract`, `model`, the Model reference form, `output`, `usage`, `tokens`, `input_tokens`, `output_tokens`, `price`, `amount`, `currency`, `ExtractionIncompleteError`, `details`, `missing`, `path`, `ExtractionInvalidError`, `issues`, `message`, `finish`; must close (`async with`), must await.
- Goodfetch Rust, 36: `schemars` (the re-export), `Collie`, `scope`, `ConnectOptions`, `Error`, `start`, `new_page`, `goto`, `expect`, `to_have_title`, `Expected`, `regex`, `extract`, `Schema`, `of`, `model`, the Model reference form, `output`, `usage`, `tokens`, `input_tokens`, `output_tokens`, `price`, `amount`, `currency`, `ExtractionIncomplete`, `details`, `missing`, `path`, `ExtractionInvalid`, `issues`, `message`, `finish`; must await (builders), must close (`scope`), must use the Run report (`#[must_use]`). `Result::inspect_err` is the standard library's.

**Failure points, scored**

| Failure point | Stagehand (TypeScript) | Goodfetch (all three) |
|---|---|---|
| 1. Answer of the wrong type (`"£51.77"`, `"Three"`) | generic: a `ZodError` thrown after the call, whose issues hold the path; no retry with the errors fed back, and the call's usage is lost | typed: `ExtractionInvalidError` after ADR 0012's bounded retries with the errors fed back; `details.issues` per field, with the model and the strategy, and `usage` on the error |
| 2. A field absent from the page | silent: nothing checks a value against the page, so the model's empty string, `0` or invented text passes the schema | typed: `ExtractionIncompleteError`, `details.missing` naming each path and its reason (`absent`, `no_match`), `found` holding the rest |
| 3. Provider unreachable, refusing or rate limiting | generic: the runtime's error as text, or the local callback's own `Error` | typed: `ModelUnavailableError` (`reason`, `retryAfter`, retryable), `ModelDeclinedError`, `ModelUnauthorizedError` or `ModelFundsExhaustedError`, each naming the Model reference |
| 4. No structured output from the model | generic: the provider's rejection as text; locally, `JSON.parse`'s `SyntaxError` in the program's callback | typed: `ai.structured_output: auto` steps down from native to a forced tool call to the schema in the prompt, recorded in the Run trace; past the last rung, `ModelRequestRejectedError` or `ExtractionInvalidError` naming the model and the strategy |
| 5. Page not loaded, or another page | generic: `goto`'s failure is an `Error` as text, and so is the program's own title check, two lines on `page.title()`, for an error page or another book | typed: `NavigationFailedError` (`netError`, retryable); `SuccessCheckFailedError` from the title check, with what it expected and observed |
| 6. Usage or cost not reported | silent: tokens only, priced by the program's own table, which goes stale without notice and cannot price another model; a call that throws reports nothing | typed: `usage` on every result and every error, priced from the price table; a model it cannot price is `null`, printed "unpriced", never a guess (ADR 0101) |

**Verdict.** Goodfetch TypeScript needs 13 fewer concepts and 35 fewer lines than Stagehand's: a local model is one more Model reference (`ollama/qwen3:8b`) rather than a callback against the server's API, and the price comes with the usage rather than from a table the program keeps (ADR 0101, G6). Both check the title; Goodfetch's `expect` turns an error page into a typed failure where Stagehand's check throws its own text, and Goodfetch adds `finish()`, which ends the Run with its report (G1, ADR 0102). Goodfetch Python is 13 lines longer than its TypeScript twin, ruff splitting the long calls and the error handlers. Every Goodfetch failure point is typed, where Stagehand's are generic or silent: a missing field is checked against the page and a wrong type is retried with the errors fed back (ADR 0091, G1). Rust needs the most concepts of the three Goodfetch programs, from the schema derive, `Expected` and the `#[must_use]` report (S1 in Rust); it has no scored leader.

## After the sketch

A new Journey on the SDK surfaces (J-011 covers extraction without a model); Proving ground families: extraction and AI, served by the Fake model.
