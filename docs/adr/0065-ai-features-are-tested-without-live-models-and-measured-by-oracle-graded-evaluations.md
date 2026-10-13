# AI features are tested without live models, and measured by Evaluations the oracle grades

No test of an AI feature calls a live model. A **live model** here is one a provider serves and Goodfetch doesn't pin: a cloud model, reached directly or through a gateway. A local model at a pinned digest on a pinned server is a test input, not a live model. Goodfetch's AI features (Plans and Repair, `act` and `observe`, AI extraction, Success checks, the Agent loop, Code mode, Decision providers) and its agent skills are tested against three stand-ins: a **Scripted model** in-process, the Proving ground's **Fake model** over the wire, and **Model recordings** of real provider traffic replayed at each adapter's base URL. Live models are reached only by Live checks (ADR 0064, pending in [Flakiness policy and checks that are non-deterministic by nature](https://github.com/Jbmanllr/automan/issues/280)): a provider check, a recorder, and **Evaluations**, which measure each AI capability on Proving ground tasks, graded by the Proving ground's oracle and never by a model, under ADR 0025's statistics.

Every framework surveyed tests its own logic with a scripted model and no network: the Vercel AI SDK's `MockLanguageModelV4`, LangChain's fake chat models, the OpenAI Agents SDK's `ScriptedModel`, Pydantic AI's `TestModel`. None of the Vercel AI SDK, OpenAI Agents SDK, Anthropic SDK or Skyvern CI passes a real model key. Wire formats are tested against responses recorded once: the AI SDK serves real-response fixtures from a local server, Pydantic AI fails a test whose cassette was only partly used, and LangChain replays its cassettes playback-only on every pull request, matching on the body. Recorders that patch HTTP clients break: vcrpy's default matching ignores the body, and Anthropic's SDK v1 warns that vcrpy and respx "can silently fail". Determinism can't come from the providers: current Anthropic models return 400 on a non-default `temperature`, Gemini ignores its sampling parameters from 3.6 Flash, and OpenAI's `seed` is best effort. LLM judges agree with people only partly (Online-Mind2Web), and ADR 0013 never lets a model alone judge success.

Decided in [Testing AI features and agent skills](https://github.com/Jbmanllr/automan/issues/281), answered by the agent on the owner's delegation, on the facts of [Testing AI features without flaky tests](https://github.com/Jbmanllr/automan/issues/268) ([findings](https://github.com/Jbmanllr/automan/blob/research/ai-testing/rnd/research/2026-10-13-ai-testing.md)) and [Testing MCP, framework adapters, Integrations and agent skills](https://github.com/Jbmanllr/automan/issues/267) ([findings](https://github.com/Jbmanllr/automan/blob/research/agent-surface-testing/rnd/research/2026-10-13-agent-surface-testing.md)). The rules every test follows are in `TESTING.md`.

The suites are the **Quick**, **Pre-merge** and **Release** suites of ADR 0061 (pending in [Test suites, local commands and time budgets](https://github.com/Jbmanllr/automan/issues/276)), each containing the one before, and the CI tiers are ADR 0032's: pull request, merge queue (which runs the Release suite), nightly and the release pipeline. Live checks are in no suite (ADR 0064).

## Decision

### Three stand-ins for a model

- **The Scripted model** is a test adapter of the engine's Model provider interface (ADR 0012), never compiled into the binary. It answers call *n* with reply *n* (text, a tool call, structured output, a stream, or a provider error), fails the test on a call beyond its script and on steps left unused, records every request, and reports token usage from its script. In-process tests of Repair, JSON repair and the bounded retry, the Agent loop, Code mode and its journal replay (ADR 0051), Decision provider abstention, resume from a journaled answer (ADR 0010), Usage and currency Limits run on it, in the Quick suite.
- **The Fake model** is the lab's `fake-llm.ts`, ported to the Proving ground by area 2 and extended:
  - it speaks the three formats of ADR 0012 (OpenAI-compatible chat, Anthropic Messages, Gemini), streamed and not, with tool calls and structured output;
  - its answers depend only on the request and the Proving ground seed;
  - it counts each call's tokens itself, so a test checks the engine's Usage against an independent count;
  - a task can ask it for a fault: a rate limit with `Retry-After`, an overload, a timeout, a cut stream, malformed JSON, a refusal, or a wrong but plausible answer that a Success check or Grounding must refuse.
  End-to-end tests and AI journeys use it through the real binary, the real adapters and a real Chromium, in the Pre-merge suite.
- **SDK suites** script models through the SDK-handler form of the Model provider Plugin kind (ADR 0033), which is a shipped feature, not a test hook.
- **Area 2's replay proxy keeps its meaning:** it serves archived real pages to browsers. It is not a model recorder.

### Model recordings

- **What and where.** Real exchanges with a Model provider, recorded once, scrubbed, committed beside the adapter's tests and replayed by `httpmock` at the adapter's base URL, so the real adapter code runs with no client patching. Playback is in the Pre-merge suite, since it binds a local socket. The checks on the recording files are in the Quick suite.
- **Matching.** On method, path and the canonical JSON body. A request with no recording fails, and so does a recording left unused.
- **Streams** replay whole and also re-cut at seeded boundaries (ADR 0059's `Entropy`). The stream parsers also get `proptest` and a fuzz target.
- **Coverage** is a fixed matrix, and a missing cell fails a check:
  - per format on the direct provider: text, a tool call and its result turn, parallel tool calls, structured output, streaming of each, usage including cached and reasoning tokens, prompt caching, provider search grounding, image input, and the errors the engine branches on;
  - the OpenAI-compatible format through the gateways of ADR 0012: text, a tool call, structured output and one error;
  - each hosted Decision provider of ADR 0009 (Jev and OpenAI's Decisions API): a choice, an abstention, a low-confidence choice, and the errors the engine branches on.
- **Redaction** keeps an allowlist at write time: request headers `content-type` and the version and beta headers the adapter sends on purpose, response headers `content-type`, `retry-after` and the rate-limit headers the adapters read. Key query parameters are removed, and request and organization IDs are replaced. Prompts come only from Proving ground pages. A check fails on any other header or anything shaped like a provider key.
- **Refresh** happens on cause, never on a timer and never automatically: when the engine's request changes, when the provider Live check reports a change, or 30 days before a model's announced retirement, which a manifest of every model ID in use makes visible. A manually dispatched recorder job holds the keys and opens a pull request, reviewed as a diff.
- **Synthetic faults** that aren't worth recording are written as explicit `httpmock` stubs, marked as synthetic.

### Requests and assertions

- Every AI operation has an `insta` snapshot of the whole request it sends, per format, so a prompt or tool-description change is a reviewed diff. Quick suite.
- A test of an AI feature asserts outcomes Goodfetch owns: the Success check result, Grounding, the Plan file, the Effect journal, the Record, the Error code, Usage and what the oracle saw. It never asserts a model's wording, and no test relies on temperature or a seed.

### Pinned local models and the in-engine model

- **Local model servers run for real, in the Release suite's Linux cell:** llama.cpp server and Ollama, pinned, each with a pinned small model on CPU. The model is downloaded by digest before ADR 0043's deny, like a pinned image. Tests assert protocol behaviour only: a parsed answer, a tool call, structured output valid against its schema, usage, a stream that ends. The engine sends every sampling parameter it relies on in each request, and results record the server version and model digest.
- **vLLM** runs in the same cell on its CPU build where its pinned version serves the model on CPU. Otherwise it runs in the release pipeline on a GPU host, which the owner approves as a paid test resource.
- **LM Studio** runs in the release pipeline where a runner supports its headless server, and is a manual test otherwise.
- **ADR 0012's tested list.** In the release pipeline, every model on the list runs the same protocol tests and the small local task set. A model that fails leaves that release's list.
- **The in-engine Decision provider model** is pinned by digest. Its choices are checked on a labelled set with a stated margin, on Linux in the Pre-merge suite and on every OS in the Release suite. Confidences are never compared exactly.

### Live checks that reach live models

Each is a Live check of ADR 0064, owned by area 9, with the placement below.

| Live check | Tier | What it blocks |
|---|---|---|
| Provider check | nightly | nothing directly |
| Recorder | manual dispatch | nothing: it opens a pull request |
| Pull-request Evaluation | pull request, prompt-affecting paths only | that pull request, on an absolute rule (below) |
| Nightly and weekly Evaluations | nightly | the next release candidate, on an absolute rule; on a regression only once the owner approves Δ |
| Skill Evaluations | nightly (weekly), and pull requests that change skills or catalog descriptions | as the Evaluations |
| AI campaigns of the release benchmark | release candidate | the release, by ADR 0025's rule |

- **Provider check.** It covers each format on its direct provider, each named gateway and each hosted Decision provider. It sends each recorded request live with the provider's cheapest current model and compares the response's shape with the recording: fields, types, stream event kinds and order, error bodies, and the headers the adapter reads. It never compares content. It also lists each provider's models against the manifest. Each difference opens or updates its `live-check` issue.
- **Gating a merge on a Live check** is declared here as ADR 0064's table asks. If ADR 0064 as merged lets no Live check block a merge, the pull-request Evaluation reports only.

### Evaluations

- **One evaluation set, in `bench/`.** Campaigns built on ADR 0025's statistics and oracle run at three sizes: the pull-request size, the nightly size (with a weekly run on larger models), and the release benchmark of ADR 0025, which becomes the largest run of the same campaigns.
- **Capabilities covered.**
  - Plans and Repair: Plan compilation, replay with Drift and Repair, `act` and `observe`.
  - Extraction: AI extraction to a schema with Grounding, per-page extraction, extraction rules written once.
  - The Agent: the built-in Agent on objective tasks, login recovery and extraction discovery.
  - Crawls driven by AI: a crawl configured from a prompt, AI-guided crawl, search then crawl.
  - Code mode, and vision against DOM actions.
  - Decision provider choices and calibration, in-engine and hosted.
  - AI safety: prompt injection, fake success messages, allowlist escapes, destructive look-alikes.
  - Outside Agents through MCP, the CLI with skills, and Code mode.

  Fleet, MFA rotation and hosted-browser use cases run at the release size only.
- **Grading.** Only the oracle grades a trial: verified, false success, `needs-review` confirmed or not, failed. The engine's Success check and Grounding results are recorded and compared with the oracle. No model grades an Evaluation. After a regression, a maintainer reads the failed trials' Run traces; that is a manual test.
- **Statistics.**
  - Each task runs 3 trials per model at the pull-request and nightly sizes, each from a clean Goodfetch home. Every trial counts; a failed trial is never replaced (ADR 0064).
  - Reported: the verified rate with its 95% Wilson interval, pass^3, false successes, safety violations, tokens and dollars per verified result, and the Success check and Grounding disagreements with the oracle. A report states the bound that "zero" can claim at its trial count (about 3/*n*).
- **Absolute rules.** A false success or a safety violation fails an Evaluation at any size. It is a correctness failure, so it blocks the next release candidate, as ADR 0025 blocks a release on any correctness failure.
- **Regression rule.**
  - **Statistic:** the mean, over tasks, of the paired difference in verified rate between the run and the last accepted baseline on the same pinned models. It is an absolute difference in percentage points, with ADR 0025's block-bootstrap interval. The lab's log-ratio form is not used, because it is undefined when a task's rate is 0.
  - **Verdict:** the rule fails when the interval's upper bound is below −Δ. An inconclusive result is reported, never a failure.
- **Δ is proposed, not yet approved.** The proposal is 10 percentage points per capability, then the value measured from the run-to-run spread once ten nightly runs exist. ADR 0025 has the owner approve every release-gate Δ. Until the owner approves this one, a regression opens or updates its `live-check` issue under ADR 0064's rules and blocks no release candidate.
- **Capability and regression sets.** A task joins its capability's regression set after pass^3 ≥ 0.9 on every nightly model for five nights in a row. A regression set that passes everything for 30 nights gets harder tasks. Tasks use development seeds at the pull-request size and held-out seeds at the nightly and release sizes, so prompts aren't tuned on the tasks that judge them. Every AI bug fixed adds a task.
- **Prompt-affecting paths** are prompts, request snapshots, catalog descriptions and skills. Fork pull requests never receive model keys.
- **Models.** A manifest pins one small and one larger model per format by dated or versioned ID. A model change runs old and new side by side for five nights before the old one is dropped. Every result records the model, any fingerprint the provider returns, the gateway, the local server's version and model digest, the commit, the seed and the prompt snapshot hashes.

### Agent skills and tool descriptions

- **Skills, with no model:**
  - in the Quick suite, `xtask codegen --check`, and the open Agent Skills spec's frontmatter limits enforced by the generator's tests;
  - in the Pre-merge suite, a check that every operation, command and option a skill names exists in the catalog, and a run of every example command in a skill against the local Proving ground, checked by the oracle.
- **What the skills package is** belongs to area 11. **Assumed interface:** if area 11 ships the skills as a Claude Code plugin, `claude plugin validate --strict`, at a pinned Claude Code version, joins the Pre-merge suite on that plugin's manifest. Whatever else it ships them as gets that format's own validator.
- **Skill Evaluations** run through promptfoo, called from a `bench/` campaign:
  - a coding agent (Claude Code through the Claude Agent SDK, on the small, mid and large models the skills target, and Codex) runs the CLI against the local Proving ground, with the skills and without them, 3 trials each;
  - graded by the oracle, with promptfoo's `skill-used` reported as a signal of triggering;
  - a skill whose with-skills rate is no better than its baseline on some model opens an issue.
- **Tool descriptions:**
  - in the Quick suite, an `insta` snapshot of the catalog's MCP projection per profile, each description at most 2,048 characters, and each profile's total tool-definition size recorded in the snapshot, so growth is a reviewed diff;
  - a tool-selection Evaluation that checks a model's first tool call against the expected call, without executing it.

### Spend

- **Every live job has its own credentials and a hard cap outside Goodfetch**, so a cap holds even for model calls the engine doesn't make (ADR 0012 leaves an outside agent's calls outside the engine):
  - Anthropic: one workspace per job, with the workspace's monthly spend limit set to the job's ceiling.
  - Gateways: one key per job, with the key's own spend limit (OpenRouter's per-key `limit`, or the gateway's equivalent).
  - OpenAI and Gemini direct: one key per job, under the provider's project budget. The Test infrastructure spec confirms that each budget stops calls. Where one doesn't, that job reaches the provider through a capped gateway key instead.
- **Each job also stops itself at its per-run cap:**
  - Evaluations of the engine run under a currency Limit equal to the cap (ADRs 0012 and 0041).
  - Claude Code trials run with the Claude Agent SDK's `maxBudgetUsd`.
  - Codex reaches its models through a capped key, set by its `model_providers` base URL where needed.
  - The provider check and the recorder send a fixed list of requests with bounded output. Their worst case is computed from the price table before the first call, and they refuse to start above the cap.
- **Each job prints its planned trials, models and worst case before its first call.**
- Batch APIs are not used, because the engine doesn't call them, so an Evaluation through them would measure a path users don't run.
- **Proposed budget, which the owner approves before anything is spent:**

| Job | Cap per run | Monthly ceiling |
|---|---|---|
| Provider check | $0.50 | $15 |
| Recorder | $2 | $10 |
| Pull-request Evaluations | $2 | $60 |
| Nightly Evaluations | $4 | $120 |
| Weekly Evaluations and skill Evaluations | $35 | $150 |
| **Total** | | **$355** |

  ADR 0025's AI campaigns cost up to $150 per release candidate, approved per release with the benchmark's other paid services.
- Until the owner approves, every live job is present but disabled by a repository variable.

## Considered Options

- **Live models in the test suites, at temperature 0 or with a seed:** rejected. Providers no longer offer determinism, and tests that expect model output fail at random (spiceai #6399, effectful #773).
- **Recording by patching HTTP clients** (vcrpy-style, `cassetter`, `rvcr`): rejected. Patching is where recorders break, and `cassetter`'s Rust wrapper is not a `reqwest::Client`, so it doesn't plug into async-openai. A replay server at a base URL needs no patching.
- **A hand-written replay server:** rejected. `httpmock` already records through a proxy and plays back.
- **Hoverfly** for its diff mode: rejected. A Go proxy in the Rust suites for one feature the provider check already covers.
- **Refreshing recordings on a timer, or automatically when a live check differs:** rejected. A timer refreshes what didn't change, and an automatic refresh turns a provider regression into a green test.
- **A denylist of secret headers:** rejected. It leaks the next header nobody listed.
- **An LLM judge, even as a tie-breaker** (Stagehand, Browser Use, Inspect AI's `cascade()`): rejected. The Proving ground's expected results come from its seed, so no task needs one, and judges agree with people only partly.
- **ADR 0025's release benchmark as the only Evaluation:** rejected. A prompt regression or a provider change would surface once per minor release.
- **A separate evaluation framework for everything** (Inspect AI, promptfoo): rejected. It would be a second statistics engine and a second grading path next to `bench/`. promptfoo is kept only to drive coding agents.
- **`claude plugin eval` for skills:** rejected as the runner. It covers Claude Code only, with its own case format. promptfoo drives Claude and Codex.
- **A fixed pass-rate gate** (Stagehand's 0.8): rejected. It ignores sample size and noise.
- **Evaluations on every pull request:** rejected. They add cost and non-determinism to every merge, for changes that touch no prompt.
- **A currency Limit as the only spend fence:** rejected. It covers only the engine's own model calls, not Claude Code's or Codex's.

## Consequences

- **Amends `TESTING.md`'s Dependencies in tests (ADR 0062).** Its "True external" category now names the stand-ins for Model providers: the Scripted model in-process, Model recordings for wire formats, the Fake model end to end. The rules are in its AI features and agent skills section.
- **Amends ADR 0043.** Its list of jobs that may reach named endpoints gains these jobs, each reaching only the API hosts of the Model providers, gateways and hosted Decision providers it names:
  - the provider check;
  - the recorder;
  - the pull-request, nightly and weekly Evaluations;
  - the skill Evaluations.

  Their Proving ground is the local copy, under the deny. Claude Code runs with `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` and Codex with its update check and analytics turned off, so neither calls a host the job doesn't name. Pinned model weights are fetched before the deny, with the dependency installs. A one-line note marks the list.
- **Adds to ADR 0032.** The pull-request tier runs the pull-request Evaluation on prompt-affecting paths. The nightly tier runs the provider check, the Evaluations and their weekly run. The release pipeline runs the tested local models, and vLLM or LM Studio where they need it. While the repository is private, ADR 0032 keeps its nightly tier off and runs its full matrix on manual dispatch, so these nightly Live checks also run on manual dispatch. The pull-request Evaluation runs in the pull-request tier, which runs while the repository is private, once the owner approves the budget.
- **Adds to ADR 0025.** Its AI use cases become campaigns that also run at the pull-request and nightly sizes, under its oracle, statistics and three views. A false success found at any size blocks the next release candidate as a correctness failure. A regression blocks it only once the owner approves the Evaluations' Δ.
- **Adds to ADR 0012.** Every adapter takes a base URL. The engine sends every sampling parameter it relies on to local servers. The tested list of local models is what the release pipeline's local-model run passes.
- **Adds to ADR 0009.** The calibration Evaluation is the measurement on unseen pages that a Decision provider's threshold needs before it is used. Hosted Decision providers get Model recordings and the provider check.
- **Adds to ADR 0011, for area 11.** The skill generator enforces the Agent Skills frontmatter limits. Skills name only catalog operations, and their example commands run. Each catalog description is at most 2,048 characters.
- **ADR 0013 is unchanged:** no model grades a test or an Evaluation either.
- **Changes area 2's spec** ([Spec: Proving ground and conformance (V1 area 2)](https://github.com/Jbmanllr/automan/issues/152)): the Fake model speaks three formats, counts tokens, serves faults on request and stays consistent with the Model recordings. The replay proxy stays a page proxy.
- **ADR 0037's area table** lists this ADR for areas 1, 2, 9 and 11.
- **Assumed interfaces with open tickets:**
  - [Journey catalog: format, IDs, seeded data and surfaces](https://github.com/Jbmanllr/automan/issues/278): a journey can name the Fake model as its Model provider.
  - [MCP, framework adapter and Integration test stack](https://github.com/Jbmanllr/automan/issues/286): it owns the MCP Inspector checks, including `--verify` for skills over MCP and `--strict` for schemas.
  - [Security testing](https://github.com/Jbmanllr/automan/issues/288): its secret scanner also covers recordings.
  - Area 11: what the skills package ships as.
- **The "Test infrastructure" spec** builds:
  - the Scripted model's Rust interface and crate;
  - the recording format with its allowlist and matrix checks;
  - the recorder command;
  - the model manifest and its retirement check;
  - the provider check;
  - the Evaluation campaigns' small sizes in `bench/`;
  - the promptfoo setup;
  - the per-job credentials and caps.

  It also checks:
  - whether `httpmock`'s playback enforces body matching (if not, each exchange is registered as an `httpmock` mock with a JSON body matcher);
  - whether OpenAI's and Google's project budgets stop calls;
  - LM Studio's headless server on CI runners;
  - vLLM's CPU build;
  - that Claude Code and Codex, so configured, reach no host but the named model hosts.
- **Spend waits for the owner:** the budget and Δ. Until the budget is approved, only the model-free layers run.
