# 03. Repeat AI steps with no model call

## Task statement

> On https://quotes.toscrape.com/, using steps written in plain words rather than selectors: open the tag "love" from the tag list, go to its second page, and open the "about" page of the first quote's author. Before returning, confirm without a model that the page reached is an author page (its address starts with https://quotes.toscrape.com/author/). Return the author's name, birth date and birthplace. Keep what the model resolved for each step in a file next to the program. A second run must repeat the same steps with no model call at all. If the site changes so that one kept step no longer applies, only that step goes back to the model, and the program says which step that was.

## Target

`quotes.toscrape.com`: the tag cloud, `/tag/love/page/2/` and author pages such as `/author/Albert-Einstein/` with their birth date and place (checked 2026-10-14).

## Leader

Stagehand 4.2.1, `act` replaying the Actions `observe` resolved, kept in the program's own file (TypeScript). Stagehand 4's cache is kept on Browserbase's server and has no effect on a local browser (verified in 4.2.1's types and docs, below); Stagehand 3.7.3 kept a local cache folder, noted in the comparison.

## Pains

P03 replay without the model, P26 local-first artifacts, P04 verified rather than claimed, P36 reviewable output; keeps K02 generate once, then own the deterministic artifact.

## Failure points

1. A step resolves to the wrong element but the run continues (a plausible wrong result).
2. A kept step no longer matches the page (Drift).
3. The model is unavailable on the first run.
4. A kept step drifts on a later run that has no model configured.
5. The end state is not reached, and the confirmation fails.
6. The kept file comes from an older version of the library.

## Setup

Counted apart from lines. Each version needs the model's key for its first run and for a step sent back to the model; a replay needs none. Goodfetch's engine comes with each package and its pinned browser is fetched on first use (ADRs 0024, 0057); Stagehand 4 launches the Chrome installed on the machine, or the one `CHROME_PATH` names.

| Program | Commands | Count |
|---|---|---|
| Stagehand (TypeScript) | `npm install @browserbasehq/stagehand@4.2.1`; Google Chrome installed; `export ANTHROPIC_API_KEY=…` | 3 |
| Goodfetch TypeScript | `npm install @goodfetch/collie zod`; `export ANTHROPIC_API_KEY=…` | 2 |
| Goodfetch Python | `pip install goodfetch-collie pydantic`; `export ANTHROPIC_API_KEY=…` | 2 |
| Goodfetch Rust | `cargo add goodfetch-collie serde --features serde/derive`; `cargo add tokio --features macros,rt-multi-thread`; `export ANTHROPIC_API_KEY=…` | 3 |

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Stagehand (TypeScript) | 28 | 44 | 3 | 0 / 5 / 1 |
| Goodfetch TypeScript | 24 | 25 | 2 | 6 / 0 / 0 |
| Goodfetch Python | 24 | 36 | 2 | 6 / 0 / 0 |
| Goodfetch Rust | 35 | 53 | 3 | 6 / 0 / 0 |

Programs: [`leaders/03-replay-ai-steps.stagehand.ts`](leaders/03-replay-ai-steps.stagehand.ts), [`../typescript/scenarios/03-replay-ai-steps.ts`](../typescript/scenarios/03-replay-ai-steps.ts), [`../python/scenarios/s03_replay_ai_steps.py`](../python/scenarios/s03_replay_ai_steps.py), [`../rust/examples/s03_replay_ai_steps.rs`](../rust/examples/s03_replay_ai_steps.rs). Every program uses the same three instructions and the same check, and keeps its file next to itself: `quotes.plan.json` for Goodfetch (the Plan, ADR 0091), `quotes.actions.json` for Stagehand.

What was verified about the leader, in the installed packages:

- **Stagehand 4.2.1's cache is Browserbase's.** Its `cache` option is "server-side caching of act/observe/extract results … Requires a Browserbase apiKey and browser sessionId" (`StagehandInitParamsSchema` in `dist/index.mjs`), and the docs say it "has no effect on local browsers" (docs.stagehand.dev/v4/basics/observe). Its program therefore keeps each Action that `observe` resolved in its own file and replays it with `act(action)`, which "skips model inference" (docs.stagehand.dev/v4/basics/act).
- **Its Repair is `selfHeal`.** In the bundled runtime (`dist/extension/service-worker.js`), a replayed Action is re-inferred once only when its selector fails to perform, from the Action's description rather than the program's instruction; the result carries the new selector in `data.actions`, which is how the program tells which step went back to the model. A failure, healed or not, is `success: false` with a message, never an exception.
- **`extract` has no local replay**, so the program reads the author's three fields with selectors to keep the second run free of model calls.
- **Stagehand 3.7.3 kept a local cache folder**: `cacheDir?: string` on its constructor options (`dist/esm/lib/v3/types/public/options.d.ts`), files the program does not write itself. Stagehand 4 dropped it for the server cache.

**Concepts counted** (each name once per program; obligations last):

- Stagehand (TypeScript), 28: `Action`, `localBrowser`, `launch`, `Stagehand`, `create`, `browser`, `model`, `modelName`, `apiKey`, the `"anthropic/…"` name form, `selfHeal`, `context`, `pages`, `goto`, `observe`, `data`, `act` (with an Action, its replay form), `success`, `message`, `actions`, `selector`, `url`, `locator`, `textContent`, `close`; must close Stagehand, must close the browser, must await. Node's `fs` and JSON are the standard library's.
- Goodfetch TypeScript, 24: `Collie`, `connect`, `settings`, `ai.model`, the Model reference form, `start`, `plan` (the Plan file), `newPage`, `goto`, `act`, `step`, `source`, `"repair"`, `instruction`, `expect`, `toHaveURL`, `extract`, `output`, `finish`, `plan` (the report's `PlanReport`), `status`, `changedSteps`; must close (`await using`), must await.
- Goodfetch Python, 24: `AsyncCollie`, `connect`, `settings`, `ai.model`, the Model reference form, `start`, `plan`, `new_page`, `goto`, `act`, `step`, `source`, `"repair"`, `instruction`, `expect`, `to_have_url`, `extract`, `output`, `finish`, `plan` (the `PlanReport`), `status`, `changed_steps`; must close (`async with`), must await.
- Goodfetch Rust, 35: `schemars` (the re-export), `settings::ai`, `MODEL`, `Collie`, `ConnectOptions`, `settings`, `Settings`, `new`, `set`, the Model reference form, `scope`, `Error`, `start`, `plan`, `new_page`, `goto`, `act`, `step`, `source`, `StepSource`, `Repair`, `instruction`, `expect`, `to_have_url`, `Expected`, `regex`, `extract`, `Schema`, `of`, `output`, `finish`, `plan` (the `PlanReport`), `status`, `changed_steps`; must await (builders), must close (`scope`). `concat!` and `env!` are the standard library's.

**Failure points, scored**

| Failure point | Stagehand (TypeScript) | Goodfetch (all three) |
|---|---|---|
| 1. A step resolves to the wrong element | generic: the program's own URL test throws an `Error` as text, and the program writes its file only after that test, so a wrong step is not kept | typed: `SuccessCheckFailedError` with the check, expected and observed; the Run is `rejected` and the compiled steps stay `pending_review`, never replayed (ADR 0091). In every program, a wrong step that still ends on some author page passes the task's own check |
| 2. A kept step no longer matches (Drift) | silent: only a selector that fails to perform is healed; one that still resolves to a changed element is replayed without notice | typed: each step's targets are checked against their Target fingerprints; that step alone gets one Repair, `step.source` reads `repair` with its `drift`, and the report's `changedSteps` names it |
| 3. Model unavailable on the first run | generic: `observe` throws the runtime's error as text | typed: `ModelUnavailableError` (`reason`, `retryAfter`, retryable), or `ModelNotConfiguredError` naming `ai.model` |
| 4. A kept step drifts with no model configured | generic: the heal's inference fails and `act` returns `success: false` with a message, which the program throws as its own `Error` | typed: `PlanDriftError` naming the step, its `drift` and `whyNoRepair` (`no_model`, or `frozen` under `plan.frozen`) |
| 5. End state not reached, confirmation fails | generic: the program's own `Error` with the URL; its file is left as it was | typed: `SuccessCheckFailedError`; the Run is `rejected` and no new Plan version is written |
| 6. The kept file is from an older library version | generic: the file is the program's own JSON of Actions with no version, so a changed shape fails the protocol's strict `Action` schema with a `ZodError` | typed: `PlanInvalidError` naming the file and the problem; the Plan format is versioned and read under ADR 0030 |

**Verdict.** Goodfetch TypeScript needs 4 fewer concepts and 19 fewer lines than Stagehand's: the Plan file, its replay with no model call, Drift detection and a Repair limited to the drifted step are one `plan` option, where Stagehand's program reads, compares and writes its own file because its cache does not work locally (ADR 0091, P26). Goodfetch's extraction replays from the Plan too (compiled mode), where Stagehand's program falls back to selectors to keep the second run free of model calls. Goodfetch adds the model setting at the connection and `finish()` with its `PlanReport`, which say how each step was resolved and what became of the Plan (G6, ADR 0102). Every Goodfetch failure point is typed, where Stagehand's are generic and a selector that still resolves after a redesign is replayed silently (G1). Rust needs 11 more concepts than its TypeScript twin, from the setting constants, `StepSource` and `Expected` (S1 in Rust); it has no scored leader.

## After the sketch

A new Journey on the SDK surfaces; Proving ground families: AI, and repeated runs with Drift.
