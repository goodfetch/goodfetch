# 11. Reach a goal from one instruction, with limits and a verdict

## Task statement

> On https://books.toscrape.com/, from one instruction, "find the cheapest book rated five stars in the Travel category and give its title and price", reach the answer without writing the steps yourself. Cap the work at 25 steps and 0.20 USD. Return the title and the price as typed fields, together with how the work ended: stopped by a limit, reported as done by the model, or verified by a condition the program states (the page reached is a book page in the Travel category with a five-star rating). Keep the steps taken so that they can be repeated without a model. Stay on books.toscrape.com, whatever the pages suggest. Nothing about the task may be sent to any third party other than the chosen model's provider.

## Target

`books.toscrape.com`: the Travel category and its book pages (checked 2026-10-14).

## Leader

Browser Use 0.13.11 (Python): `Agent(task=..., llm=...)` and `run()`.

## Pains

P04 false success, P42 budgets and cost, P25 lean observations, P19 model breadth, P13 telemetry, P15 domain policy, P03 replay without the model; keeps K08 any model and K10 token-efficient observations.

## Failure points

1. The model reports success with a wrong answer.
2. A step or cost limit is reached: the partial answer and the reason are returned.
3. The loop repeats itself without progress.
4. The model provider is unreachable.
5. A page's observation is too large for the model.
6. Task text or addresses leave the machine to a party other than the model's provider.
7. A page leads the model off the site.

## Setup

- **Browser Use (Python):** `uv pip install browser-use==0.13.11`, `uvx browser-use install` (Chromium), `export ANTHROPIC_API_KEY=…`. Telemetry is switched off in the program (`ANONYMIZED_TELEMETRY=false`, before the first run), which also turns cloud sync off.
- **Goodfetch TypeScript:** `npm install @goodfetch/collie zod`, `export ANTHROPIC_API_KEY=…`. The pinned browser is fetched on first use (ADRs 0024, 0057); there is no telemetry to switch off (ADR 0043).
- **Goodfetch Python:** `pip install goodfetch-collie pydantic`, `export ANTHROPIC_API_KEY=…`.
- **Goodfetch Rust:** `cargo add goodfetch-collie serde --features serde/derive`, `cargo add tokio --features macros,rt-multi-thread`, `export ANTHROPIC_API_KEY=…`.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Browser Use (Python) | 29 | 64 | 3 | 0 / 6 / 1 |
| Goodfetch TypeScript | 35 | 40 | 2 | 7 / 0 / 0 |
| Goodfetch Python | 38 | 63 | 2 | 7 / 0 / 0 |
| Goodfetch Rust | 41 | 74 | 3 | 7 / 0 / 0 |

Every program returns the title and the price with one of three endings: `stopped` (a step cap or the price cap), `reported` (the model ended the work and nothing proved it) or `verified` (the program's condition held on the page reached). Every program uses the same model, `anthropic/claude-sonnet-4-5`.

**Concepts counted.**

- Browser Use (29): `ANONYMIZED_TELEMETRY`, `Agent`, `task`, `llm`, `browser`, `output_model_schema`, `calculate_cost`, `ChatAnthropic`, `model`, `Browser`, `allowed_domains`, `keep_alive`, `run()`, `max_steps`, `on_step_end`, `token_cost_service`, `get_usage_summary()`, `total_cost`, `stop()`, `get_current_page()`, `evaluate()`, `save_to_file()`, `is_done()`, `usage`, `errors()`, `structured_output`, `is_successful()`, `kill()`; must kill the kept-alive browser.
- Goodfetch TypeScript (35): `Collie`, `connect()`, `start()`, `plan`, `limits`, `price`, `amount`, `currency`, `newPage()`, `goto()`, `agent()`, `schema`, `model`, `maxSteps`, `domains`, `successCheck`, `expect`, `locator()`, `toContainText()`, `toHaveClass()`, `finish()`, `AgentResult`, `StopReason`, `ended`, `kind`, `"stopped"`, `reason`, `partial`, `verdict`, `"verified"`, `requireVerified()`, `reported`, `claim`, `output`; must dispose (`await using`).
- Goodfetch Python (38): `AsyncCollie`, `connect()`, `start()`, `plan=`, `limits=`, `Limits`, `Money`, `amount`, `currency`, `price`, `new_page()`, `goto()`, `agent()`, `schema=`, `model=`, `max_steps=`, `domains=`, `success_check=`, `result()`, `AsyncPage`, `expect`, `locator()`, `to_contain_text()`, `to_have_class()`, `finish()`, `AsyncAgentResult`, `Stopped`, `ended`, `limit`, `reason`, `partial`, `verdict`, `"verified"`, `require_verified()`, `output`, `reported`, `claim`; must close (`async with`).
- Goodfetch Rust (41): `Collie`, `scope`, `ConnectOptions`, `start()`, `plan()`, `limits()`, `Limits`, `price()`, `Money`, `new_page()`, `goto()`, `agent()`, `schema()`, `Schema::of`, `model()`, `max_steps()`, `domains()`, `success_check()`, `Page`, `BoxError`, `expect`, `locator()`, `to_contain_text()`, `to_have_class()`, `Expected::regex`, `finish()`, `AgentResult`, `ended`, `Ended::Stopped`, `StopReason`, `partial`, `PartialOutput`, `verdict`, `Verdict::Verified`, `require_verified()`, `reported`, `claim`, `Claim`, `output`, `Error`; closed by `scope`.

### Failure points, scored

A failure point that the library rules out by design counts as typed.

| Failure point | Browser Use | Goodfetch (TypeScript, Python, Rust) |
|---|---|---|
| 1. The model reports success with a wrong answer | generic: `is_successful()` and the default judge are models' words; the program's own check runs after the run, as a boolean, and never goes back to the model | typed: the Success check runs at each "done" and its failure goes back to the model; the Verdict is `rejected` or `needs-review` with its reasons, and a value Grounding cannot find on the pages is named (ADRs 0013, 0100) |
| 2. A step or cost limit is reached | generic: the step cap leaves "Failed to complete task in maximum steps" as text, a price cap is the program's own hook, checked after each step, so the step that crosses it and the judge's call on "done" are spent first; no partial answer is returned | typed: `ended: stopped` with `max_steps` (naming `agent.max_steps` and its layer) or the `price` Limit, and the Grounded `partial` |
| 3. The loop repeats without progress | generic: loop detection only nudges the model; the run ends at the step cap or after 5 failures, as text | typed: `stopped` with `no_progress`, naming `agent.max_stalled_steps` (3) |
| 4. The provider is unreachable | generic: each step's provider error counts as a failure, and the run ends after 5 of them, as text | typed: `model_unavailable`, an `UnavailableError` with `retryAfter` (ADR 0101) |
| 5. An observation is too large | generic: elements are cut at `max_clickable_elements_length`; past the window, the provider's error as text | typed (by design): observations are bounded by `observation.max_size`, collapsing the deepest subtrees; past the window, `model_context_exceeded` |
| 6. Task text or addresses reach a third party | silent: PostHog telemetry is on unless `ANONYMIZED_TELEMETRY` is false, and cloud sync follows it; `calculate_cost` also fetches LiteLLM's prices from GitHub (no task data) | typed (by design): there is no telemetry, and nothing unconfigured is reached (ADR 0043) |
| 7. A page leads the model off the site | generic: `allowed_domains` is a client-side watchdog; a blocked navigation is a `ValueError` text to the model, and a redirect is caught after it loaded | typed: `agent.domains` is enforced by the engine on every navigation, the refusal given back to the model as a failed action (ADRs 0014, 0100) |

**Verdict.**

- Concepts: Goodfetch names more than Browser Use, in every language. They are the three fields ADR 0079 keeps apart (`ended`, `verdict`, `reported`) with the Grounded `partial`, and the Success check stated with `expect` (G1). Browser Use has none of these, so its program writes a price hook, a JavaScript check and string matching on the last error instead.
- Lines: TypeScript is far shorter, as the cap, the condition and the domains are options of one call. Python is as long as Browser Use's because of the `Outcome` type and the Success check function. Rust is longest, from rustfmt's line breaks and its typed `Outcome` enum.
- Setup: one command fewer in TypeScript and Python, since the browser comes on first use and there is no telemetry to switch off.
- Replay: Goodfetch's Plan replays the task with no model call (ADR 0100). Browser Use's saved history replays through `load_and_rerun()`, which still calls a model to extract and summarize.
- Failure points: Goodfetch types all seven, and Browser Use none (G1, G5, G6).

## After the sketch

A new Journey on the SDK surfaces; Proving ground family: AI, served by the Fake model. [The Agent in Collie's SDK API](https://github.com/goodfetch/automan/issues/474) answers it with `page.agent(task)` (ADR 0100): a step cap of its own, the Run's price Limit, the program's Success check, `agent.domains`, and the task kept as one Plan entry; its snippets are `snippets/agent.ts`, `snippet_agent.py` and `snippet_agent.rs`.
