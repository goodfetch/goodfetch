# Agent-writability check

[Agent-writability check](https://github.com/goodfetch/goodfetch/issues/484) of the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461) asks whether a coding agent can write correct code against the SDK API from the sketch alone. Fresh agents got each golden scenario's task statement, word for word, and the type stubs with their doc comments at commit `eb1e35b`, and nothing else. The same agents then did the same tasks on the leaders' APIs, which models already know. Its findings go to [Close the pain coverage matrix](https://github.com/goodfetch/goodfetch/issues/485).

| Path | What it holds |
|---|---|
| [`results-table.md`](results-table.md) | The summary and every run's cell, from `bin/score.py` |
| [`results.json`](results.json) | Every run's scores |
| [`runs/`](runs/) | One folder per run: the prompt (`prompt.md`), `task.json`, the program, `.checks/` (each check run's output, the program at each check, and `first-…` for the first attempt) and, for the local model, `.agent/` (its transcript) |
| [`judgments/`](judgments/) | Each judge's prompt and verdicts: `NN.a.*` for the Claude models, `NN.b.*` for the local model |
| [`bin/`](bin/) | The harness: `setup.sh`, `make_runs.py`, `check.py`, `check_all.py`, `ollama_agent.py`, `run_local.sh`, `make_judge.py`, `sync.sh` and `score.py` |

## Models

No paid resource was used (ADR 0076), and no money was spent.

- **Claude Sonnet** and **Claude Haiku**, as fresh Claude Code subagents (`model: sonnet` and `model: haiku`), one agent per run and at most two at a time.
- **qwen2.5-coder:14b** (Q4_K_M, through ollama 0.40, 32k context, temperature 0.7), driven by `bin/ollama_agent.py`. This run is **partial: it was stopped at the owner's request**. It finished 28 of its 36 Goodfetch runs (Python 01–11, Rust 01–12, TypeScript 01–05) and none of its 18 leader runs. Of the rest:
  - TypeScript 06 and Python 12 ended on an ollama error: the model files were removed from the machine mid-run;
  - TypeScript 07 was stopped mid-run;
  - TypeScript 08–12 and the leader runs were not run.
  
  None of these counts anywhere. The machine also had qwen3-coder:30b pulled, but its 18 GB of weights could not run safely beside eight other workers on 36 GB after a crash, so the 14B model was pulled instead.
- **Judges:** Claude Opus, as fresh subagents, one per scenario and model group.
- **Not run:** OpenAI models through the Vercel AI Gateway were considered and then dropped at the owner's request. No call was made with any key; the only request was the gateway's public, unauthenticated model list.

## Method

To re-run it exactly, run `bin/` from a checkout of `eb1e35b` with this folder copied in. `setup.sh` takes the stubs from that commit, but it copies the package manifests and the Rust workspace from the working tree, and `make_runs.py` and `make_judge.py` read the scenarios from `WRITABILITY_REPO`, which defaults to the working tree. The sketch has moved on since `eb1e35b`.

- **What an agent saw.** A read-only copy of the stubs at `eb1e35b`, extracted by `bin/setup.sh`: TypeScript `collie-client`, `collie` and `foxhound` `.d.ts` files, Python `goodfetch_*` `.pyi` files, and Rust `collie-client`, `collie` and `foxhound` crates. It saw no scenarios, snippets, README, ADRs or leaders' programs.
  - Its prompt (`runs/*/prompt.md`) is the scenario's task statement, word for word, with the language and the packages available, and the rules below.
  - Scenario 09 also got the existing program. In Rust, which has no Playwright, that program launches Chromium with a debugging port, as the scenario's Setup describes.
  - Scenarios 07 and 10 got the golden programs of 05 (and 08) they build on, as #483's hand-off on #484 set for 10. 07 got them because its task starts from "the crawl of scenario 05".
  - For a leader run, the agent got the leader library's name and version, its installed package to read, and the same task statement. For 07 and 10 it also got the leader's own 05 program.
- **Rules given to every agent.**
  - Write the whole program, then type-check it with the run's `check`.
  - The first check run records the first attempt. Fix and check again, at most three check runs.
  - Never run the program, and read nothing outside the run folder and the library.
  - For the local model, `bin/ollama_agent.py` enforces these rules. Claude subagents had general tools, so for them the rules were instructions.
- **Type-check.** `bin/check.py` runs the checkers and options `check.sh` uses, on the program alone. Every Goodfetch golden program and leader program passes it unchanged (54 of 54), and a planted error fails it in each language.
  - Goodfetch TypeScript: tsc 7.0.2 with the sketch's tsconfig; Node's types are global, as they are when `check.sh` compiles all scenarios together.
  - Goodfetch Python: pyright 1.1.414 strict, plus mypy 2.4.0 `--strict` with `--enable-error-code deprecated` (#471's hand-off).
  - Goodfetch Rust: `cargo check` with warnings denied, and with the test profile for a program with tests.
  - Leaders: their own setup, tsc with the leaders' tsconfig or pyright standard with the leader's pinned packages.
- **Behaviour.** For each scenario, a fresh judge derived requirements from the task statement (and the failure points it implies). It read each final program against them: Goodfetch's by what the stubs' doc comments say a call does, a leader's by what its library does. It gave each program a verdict: correct, partial or wrong.
  - **Site facts are not scored.** No agent could see toscrape.com, so wrong selectors or URL patterns are listed apart, as `site_errors`, and do not count in the verdict. 18 of 136 programs had one. The first judge of 01 and 02 scored them before this rule; its verdicts are kept in `judgments/superseded/`, and the as-run prompts beside the final ones.
  - The local model's judges reused the Claude judges' requirements, so all three models meet one list.
- **Scores** (`bin/score.py`).
  - **Type-checks at first attempt**: the first check run passes.
  - **First-attempt correct**: the first attempt type-checks, it is the final program unchanged, and the judge found it correct.
  - **Correct in the end**: the final program type-checks and is correct.
- **Leader comparison.** The same task in the same language, on the 18 scenario-and-language pairs that have a leader program (`scenarios/README.md`), compared with Goodfetch on those same 18 pairs.

## Results

Full table: [`results-table.md`](results-table.md).

| Model | Programs | Type-checks, first attempt | Correct / partial / wrong | First-attempt correct | Correct in the end |
|---|---|---|---|---|---|
| Sonnet | Goodfetch, all 36 | 26/36 (72%) | 21 / 12 / 3 | 17/36 (47%) | 21/36 (58%) |
| Sonnet | Goodfetch, the 18 leader tasks | 14/18 | 14 / 3 / 1 | 11/18 (61%) | 14/18 (78%) |
| Sonnet | Leaders, the 18 tasks | 15/18 | 9 / 9 / 0 | 6/18 (33%) | 9/18 (50%) |
| Haiku | Goodfetch, all 36 | 34/36 (94%) | 21 / 11 / 4 | 19/36 (53%) | 21/36 (58%) |
| Haiku | Goodfetch, the 18 leader tasks | 17/18 | 13 / 4 / 1 | 12/18 (67%) | 13/18 (72%) |
| Haiku | Leaders, the 18 tasks | 13/18 | 6 / 11 / 1 | 3/18 (17%) | 6/18 (33%) |
| qwen2.5-coder:14b (partial) | Goodfetch, 28 of 36 | 2/28 (7%) | 0 / 0 / 28 | 0/28 | 0/28 |

- **Type-checks.** Both Claude models reached a passing type-check on all 72 Goodfetch programs within three check runs, 60 of them at the first attempt.
  - Of Sonnet's ten first-attempt failures, three are API shapes: Python `StepResult`'s invariance (`sonnet-gf-py-03`), a Rust `match` missing the wildcard arm of a non-exhaustive enum (`sonnet-gf-rs-11`), and unused `#[must_use]` `RunReport`s (`sonnet-gf-rs-04`).
  - The other seven are strictness or setup: Python's untyped, unknown or unused names (`py-04`, `py-05`, `py-10`), TypeScript's null checks (`ts-05`), `erasableSyntaxOnly` (`ts-12`) and a `.ts` import suffix (`ts-10`), and a Rust unused import (`rs-03`).
  - Haiku's two are Python's partially unknown types (`haiku-gf-py-03`) and a deprecation (`haiku-gf-py-07`).
- **Behaviour.** On the 18 tasks with a leader, Goodfetch programs were correct more often than the same model's leader programs: Sonnet 14 against 9, Haiku 13 against 6.
  - The leader programs' shortfalls follow the scenarios' failure points: a hosted model's cost not printed and the browser left open (Stagehand 4), the password held in clear by the program to fill it (Playwright, in all four programs), no kept steps in one Stagehand program, no recorded-crawl or AI test (Scrapy), no partial answer or stopping limit returned (Browser Use).
  - Goodfetch's shortfalls are the findings below.
- **Weaker models.** Haiku did as well as Sonnet on Goodfetch (21 correct each) and worse on the leaders.
  - The local 14B model wrote no correct program. Within 32k tokens it read one or two stub files and then invented names. Three of its Python crawl programs look for the crawl types in the Collie client package instead of Foxhound's: `AsyncCrawl` and `AsyncCrawlContext` (`local-gf-py-05`), those and `Label`, `Dataset` and `CrawlRequest` (`local-gf-py-06`), and a `Crawler` from a `_crawler` module (`local-gf-py-08`). Its Rust crawl programs import them from Foxhound. It also used a `get_by_css` page method (`local-gf-py-05`'s final program and `local-gf-py-02`'s first attempt).
  - Nine of its 28 runs never called the checker; the harness checked their final program.
- **Scenario 03's coarse check.** As #483 warned, every program uses the address-prefix Success check of the task statement, and judges did not mark it down.

## Findings for #485

Each finding is an API name or shape that agents got wrong more than once, with the runs and a proposed change. The judges' notes are in `judgments/`.

1. **No model is named, and AI verbs then fail with `model_not_configured`** (6 runs, both models, 3 languages: `sonnet-gf-py-03`, `haiku-gf-rs-03`, `sonnet-gf-ts-09`, `sonnet-gf-py-09`, `haiku-gf-ts-09`, `haiku-gf-py-09`). Agents assumed that `extract`, `act` or the Plan's first run picks a model from the provider key in the environment. `ai.model` has no default (ADR 0101), and nothing at the call site says so.
   - **Proposal:** open the doc comment of `act`, `observe`, `extract` and `agent` in every language with "needs a model: pass `model` or set `ai.model`".
   - Raise `model_not_configured` at `start()` when the Run names a Plan and no model is set, instead of at the first verb.
   - If ADR 0083 allows it, consider making `model` a required option of the verbs when the connection declares no `ai.model`.
2. **An Agent's answer is read from `output` only, so a task stopped by a limit loses its Grounded `partial`** (5 runs, both models, 3 languages: `sonnet-gf-ts-11`, `sonnet-gf-rs-11`, `haiku-gf-ts-11`, `haiku-gf-py-11`, `haiku-gf-rs-11`).
   - **Proposal:** make the Agent result a union discriminated by `ended`. `output` would exist only on `reported` or `verified`, and `partial` (with the stop reason) only on `stopped`. TypeScript and Rust then force the stopped case to be handled, and Python's `match` shows it.
3. **Images and fonts are not blocked on every browser fetch of a crawl** (3 runs, Haiku, 3 languages: `haiku-gf-ts-06`, `haiku-gf-py-06`, `haiku-gf-rs-06`). The golden program uses a before-navigation Hook on every browser fetch. `haiku-gf-py-06` and `haiku-gf-rs-06` put the route in the handler instead. `haiku-gf-ts-06` uses a before-navigation Hook but scopes it to the JavaScript and scrolling Labels, so it misses the browser fetches of the auto-tier plain Label.
   - **Proposal:** add a setting `browser.block_resources` (resource types, for example `["image", "font"]`) on a Session and a crawl, applied before every navigation.
   - Also state on `CrawlContext.page` that it is already navigated, so a route set there applies only from the next navigation.
4. **The Run is ended with `close()` instead of `finish()`, or `finish()`'s report is dropped** (4 runs: `haiku-gf-rs-04` and `haiku-gf-py-09` leave the Run stopped with a Checkpoint; `sonnet-gf-rs-04` and `sonnet-gf-rs-11` had to fix unused `#[must_use]` `RunReport` warnings).
   - **Proposal:** a scoped Run (`await using`, `async with`, `Collie::scope`) finishes when its body returns normally, and stops with a Checkpoint only on an error or a cancellation, as a transaction commits. An explicit `finish()` is then needed only to read the report, and `RunReport`'s `#[must_use]` can go.
5. **A scripted model reply in a test is given as the extracted data, while `extract.mode` defaults to `compiled`** (2 runs, Haiku: `haiku-gf-py-10`, `haiku-gf-rs-10`).
   - **Proposal:** give the testing kit a scripted reply per extract mode, for example `scriptedExtraction(data)` that answers in whichever mode is active, so a test scripts the data it expects.
6. **A fixed Run ID is treated as a label** (2 runs, Sonnet: `sonnet-gf-ts-04` starts again on an ended Run and gets `RunEndedError`; `sonnet-gf-rs-08` resumes an ended Run on a second invocation). The same family has two more shapes: `retryFailed()` called on every start (`sonnet-gf-rs-07`), and `outcomes({ kinds: ["failed"] })` read as current state (`sonnet-gf-ts-07`).
   - **Proposal:** rename the start option `id` to `runId`, with "an existing ID resumes the Run or returns it ended" as the first line of its doc comment.
   - Add a read of the requests whose latest outcome is failed (for example `run.failedRequests()`), so a program needs neither to call `retryFailed()` blindly nor to filter outcomes itself.
7. **Smaller shapes, seen once or twice:**
   - Rust: the handler's own error has no constructor, so an agent used the test-only `Error::fake` (`sonnet-gf-rs-05`). A decision on the Rust handler's error is already in progress (branch `decision/sdk-rust-handler-error`).
   - Python `StepResult`'s type parameter is invariant, so a helper typed `StepResult[object]` rejects `StepResult[Author]` (`sonnet-gf-py-03`). Proposal: make the read-only result generics covariant.
   - `SessionSource` `domains` takes registrable domains, not host names (`haiku-gf-rs-09`). Proposal: name it `registrableDomains`, or accept a host and reduce it.
   - A declared model with no configured price is refused under a currency Limit at its first call (`haiku-gf-rs-11`). Proposal: refuse it at `start()`, naming the price setting.
   - The local model's invented names point at the three-package split (ADR 0095): crawl types were looked for in the Collie client package. Proposal: a one-screen index in each package's entry file, naming where `Crawl`, `Dataset`, `Label` and `CrawlContext` live.

## Limits

- One run per model, scenario and language: a single sample, so a difference of one or two programs is noise.
- The judges are a model reading code, not a run against the site. The sketch has no implementation, so no program was executed.
- Claude subagents were told, not forced, to read only the stubs. The project's instruction files reach every Claude Code session, and they hold no scenario or answer.
- The local model ran on a loaded machine with a 32k context: its score measures a small local model in a small context, more than the API.
