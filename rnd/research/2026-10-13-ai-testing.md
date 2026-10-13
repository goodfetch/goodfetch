# Testing AI features without flaky tests

Research for [#268](https://github.com/Jbmanllr/automan/issues/268), done on 2026-10-13. Every fact below was read in a primary source on that date: official docs, source code, CI workflows, release notes, issues and papers. Versions and dates are given where they matter. "Fit for Goodfetch" notes are kept apart from the facts and are not decisions.

A word on terms. The ticket says "provider drift". In `GLOSSARY.md`, Drift means a Plan step whose target no longer matches its Target fingerprint, so this note says **provider change** for a Model provider that starts answering differently.

## Question

How can Goodfetch's AI features be tested deterministically in its suites, and how are live-model evaluations kept apart and judged? The features are Plans and Repair, natural-language actions, LLM extraction validated by a schema, Success checks, the Agent loop, model gateways and local servers, Decision providers, and Code mode.

## Summary

- **Every framework surveyed tests its own logic with a scripted model and no network.** The Vercel AI SDK ships `MockLanguageModelV4` in `ai/test`. LangChain ships `FakeListChatModel` and `GenericFakeChatModel`. The OpenAI Agents SDK has shipped `ScriptedModel` since v0.21.0 (2026-08-15), which fails on an unexpected extra model call and on unused script steps. Pydantic AI ships `TestModel`, `FunctionModel` and a global `ALLOW_MODEL_REQUESTS=False` switch. Stagehand, Browser Use and Skyvern stub the model with plain mocks.
- **Wire formats are tested against real responses that were recorded once.** The AI SDK keeps real provider responses as fixtures, served by a local test server. Pydantic AI keeps about 1,720 cassettes. LangChain replays its cassettes in playback-only mode on every PR, with a fake key. The Anthropic and OpenAI SDKs run against a mock server built from their OpenAPI spec. None of the Vercel AI SDK, OpenAI Agents SDK, Anthropic SDK or Skyvern CI workflows passes a real model key.
- **Record-and-replay tools break when they patch HTTP clients.** vcrpy's default matching ignores the request body. Anthropic's v1 SDK (2026-08-20) moved to `httpx2`, and its migration guide warns that vcrpy and respx "can silently fail". Polly.js has had no release since 2023-07, and its issue asking for Node's native `fetch` has been open since 2023-10. A vcrpy issue opened on 2026-10-08 proposes a local replay server instead of patching. In Rust, `cassetter` 0.11.0 (released 2026-09) wraps `reqwest`, and `httpmock` 0.8.3 records through a proxy.
- **Provider changes are caught by scheduled live jobs, not by PR tests.** LangChain runs live integration tests daily and refreshes model metadata daily. Pydantic AI checks its gateway model catalog weekly and runs a canary against the newest SDK versions. The AI SDK opens an issue whenever a provider's API or model list changes. Hoverfly's diff mode compares live responses with stored ones. OpenAI's `system_fingerprint` flags server-side changes.
- **Provider determinism is gone or best-effort.** Since Claude Opus 4.7, a non-default `temperature`, `top_p` or `top_k` returns HTTP 400. Anthropic's docs add that temperature 0 "never guaranteed identical outputs". Gemini deprecated those parameters on 2026-07-21: they are ignored from Gemini 3.6 Flash onward and will return 400 in later models. OpenAI's `seed` is beta and best-effort, exists only in Chat Completions, and OpenAI tells users to drop `temperature` when reasoning effort is not `none`. Locally, vLLM is reproducible only with batch invariance turned on, on the same hardware and version. llama.cpp warns that its prompt cache can make results nondeterministic.
- **CI evaluations at competitors are simple.** Stagehand runs 3 trials on internal PRs and fails below a Braintrust score of 0.8. Browser Use runs live-site tasks judged by an LLM and fails only at a 0% pass rate. The statistical tooling exists elsewhere: pass^k (τ-bench, 2024), standard errors, paired differences and power analysis (Miller, 2024), and Inspect AI's `pass_k` reducer and Wilson intervals. Anthropic's guidance (2026-01-09) is to grade outcomes, read transcripts, and move saturated capability tasks into a regression suite.
- **Users complain about three things.** First, tests that assert model wording fail at random (spiceai #6399, effectful #773, the OpenAI forum on seeds, Ollama seed issues). Second, evaluation sets rot: SWE-bench lost 68.3% of its samples to review, Skyvern marks 8 WebVoyager tasks outdated, Pydantic AI's recordings depend on a model that reaches end of life on 2026-11-30, and Stagehand fails a run closed when its judge model is retired. Third, cost: live evaluations are limited to maintainers' PRs, and recording needs keys, so contributors under-test.

## 1. Fake and scripted models, record and replay

### 1.1 Scripted models in frameworks

| Framework (version, date) | Scripted model | What it checks | Source |
|---|---|---|---|
| Vercel AI SDK (`ai@7.0.138`, 2026-10-12) | `MockLanguageModelV4`, `MockEmbeddingModelV4`, `mockId`, `mockValues` in `ai/test`; `simulateReadableStream` in `ai` | `doGenerate` takes an array of results, one per call, so a tool loop can be scripted. `model.doGenerateCalls` lets a test assert what was sent | [testing docs](https://github.com/vercel/ai/blob/main/content/docs/03-ai-sdk-core/55-testing.mdx) |
| LangChain Python (`langchain-core` 1.6.10, 2026-10-12) | `FakeListChatModel` (cycles through responses and can raise an error on a chosen stream chunk), `FakeMessagesListChatModel`, `GenericFakeChatModel` (splits messages into chunks to test streaming and callbacks), `ParrotFakeChatModel` (echoes the input) | Chain and callback logic | [fake_chat_models.py](https://github.com/langchain-ai/langchain/blob/master/libs/core/langchain_core/language_models/fake_chat_models.py) |
| LangChain JS | `FakeChatModel`, `FakeStreamingChatModel`, `FakeListChatModel` in `@langchain/core/utils/testing` | Same | [chat_models.ts](https://github.com/langchain-ai/langchainjs/blob/main/libs/langchain-core/src/utils/testing/chat_models.ts) |
| OpenAI Agents SDK Python (v0.23.1, 2026-10-02) | `ScriptedModel`, `ModelStep.respond/stream/raise_error`, `scripted_sandbox_session()`, `ScriptedRealtimeModel`, Voice fakes; in `agents.testing` since [v0.21.0](https://github.com/openai/openai-agents-python/releases/tag/v0.21.0) | "An extra model request raises `UnexpectedModelCall`; an early exit leaves steps for `assert_complete()` to report." It records each call for assertions | [docs/testing.md](https://github.com/openai/openai-agents-python/blob/main/docs/testing.md) |
| Pydantic AI (v2.55.0, 2026-10-09) | `TestModel` (procedural code that calls every tool and builds data that fits the schema; "There's no ML or AI in `TestModel`"), `FunctionModel`, `Agent.override` | `ALLOW_MODEL_REQUESTS=False` blocks any request to a real model | [docs/testing.md](https://github.com/pydantic/pydantic-ai/blob/main/docs/testing.md) |
| Stagehand (3.7.3, 2026-08-28) | `vi.fn` stub of the `generate` function | Unit tests assert the prompt and the response format sent for `act` | [act.test.ts](https://github.com/browserbase/stagehand/blob/main/packages/extension/tests/act.test.ts) |
| Browser Use (0.13.11, 2026-10-07) | `create_mock_llm(actions)`: an `AsyncMock` that returns scripted actions, then `done`; plus `pytest-httpserver` | Agent loop and events | [tests/ci/conftest.py](https://github.com/browser-use/browser-use/blob/main/tests/ci/conftest.py) |
| Skyvern (v1.0.55, 2026-10-01) | `unittest.mock`; CI sets `OPENAI_API_KEY: "sk-dummy"` | Unit tests only | [ci.yml](https://github.com/Skyvern-AI/skyvern/blob/main/.github/workflows/ci.yml) |

The OpenAI Agents SDK draws the line between the two kinds of test explicitly. Its scripted utilities are for "orchestration owned by your application and the SDK". For "Responses API or Chat Completions request serialization, authentication headers, provider defaults, HTTP payloads, provider stream chunks", the docs say: "Keep the real adapter and replace or control its network boundary." The module provides no "provider-protocol simulator" ([docs/testing.md](https://github.com/openai/openai-agents-python/blob/main/docs/testing.md), "Choose the correct boundary" and "Scope and current limitations").

### 1.2 Recorded provider traffic in the same projects

- **Vercel AI SDK.** Provider parsing tests use "test fixtures with the true responses from the providers", stored in `__fixtures__` folders. There are 61 files for the OpenAI Responses API alone. Maintainers capture them by hand: they run an example script that logs the raw body, or use `saveRawChunks` for streams. The `@ai-sdk/test-server` package (2.0.4) serves them through `createTestServer` ([contributing/testing.md](https://github.com/vercel/ai/blob/main/contributing/testing.md), [example test](https://github.com/vercel/ai/blob/main/packages/openai/src/responses/openai-responses-language-model.test.ts)). No workflow in `.github/workflows` passes a model provider key. The only secrets are GitHub, Turbo, Slack and release tokens ([ci.yml](https://github.com/vercel/ai/blob/main/.github/workflows/ci.yml)).
- **LangChain.** Standard integration tests use VCR only for selected tests, such as `test_stream_time`, behind `enable_vcr_tests`, which defaults to `False`. The base configuration uses record mode `once`, matches on `method`, `uri` and `body`, and replaces the `authorization`, `x-api-key` and `api-key` headers with `PLACEHOLDER`. An optional serializer stores cassettes as gzipped YAML loaded with `safe_load` ([conftest.py](https://github.com/langchain-ai/langchain/blob/master/libs/standard-tests/langchain_tests/conftest.py), [integration_tests/chat_models.py](https://github.com/langchain-ai/langchain/blob/master/libs/standard-tests/langchain_tests/integration_tests/chat_models.py)). On every PR, `_test_vcr.yml` "runs VCR cassette-backed integration tests in playback-only mode", using `pytest --record-mode=none -m vcr` and `OPENAI_API_KEY: sk-fake`. It "catches stale cassettes caused by test input changes without re-recording" and then fails if the working tree changed ([_test_vcr.yml](https://github.com/langchain-ai/langchain/blob/master/.github/workflows/_test_vcr.yml), [Makefile](https://github.com/langchain-ai/langchain/blob/master/libs/partners/openai/Makefile)).
- **Pydantic AI.** The repository's test guide says: "VCR + public-API tests are the default", against "real provider responses recorded as cassettes". The tree holds about 1,720 cassette YAML files ([tests/AGENTS.md](https://github.com/pydantic/pydantic-ai/blob/main/tests/AGENTS.md)). The suite records with `cassetter>=0.13.0`. A test fails when a played-back cassette was only partly used, which signals a stale tail ([tests/conftest.py](https://github.com/pydantic/pydantic-ai/blob/main/tests/conftest.py), [pyproject.toml](https://github.com/pydantic/pydantic-ai/blob/main/pyproject.toml)). It also records what the code actually sends through an httpx request hook, because "the default matchers ignore the body, so a request whose payload has since drifted still replays against its recording" ([tests/conftest.py](https://github.com/pydantic/pydantic-ai/blob/main/tests/conftest.py), `RequestCapture`).
- **Anthropic and OpenAI Python SDKs** (anthropic v1.13.0, 2026-10-09; openai v3.29.0, 2026-10-11). `scripts/mock` starts a local mock server, `steady` (`@stdy/cli@0.22.2`), built from the API's OpenAPI spec. Tests also use `respx` and `inline-snapshot`. The Anthropic CI workflow uses no model key ([anthropic scripts/mock](https://github.com/anthropics/anthropic-sdk-python/blob/main/scripts/mock), [openai scripts/mock](https://github.com/openai/openai-python/blob/main/scripts/mock), [anthropic ci.yml](https://github.com/anthropics/anthropic-sdk-python/blob/main/.github/workflows/ci.yml)).
- **OpenAI Agents SDK.** Every test job sets `OPENAI_API_KEY: fake-for-tests`. No workflow references a real model key ([tests.yml](https://github.com/openai/openai-agents-python/blob/main/.github/workflows/tests.yml)).

### 1.3 Record-and-replay tools

| Tool | Language | Version (date) | How it intercepts | Default mode | Redaction by default |
|---|---|---|---|---|---|
| [vcrpy](https://vcrpy.readthedocs.io/en/latest/) | Python | 8.3.0 (2026-07-04) | Patches HTTP client libraries | `once` ([usage](https://github.com/kevin1024/vcrpy/blob/master/docs/usage.rst)) | None. `filter_headers`, `filter_query_parameters`, `before_record_request/response` are opt-in ([advanced](https://github.com/kevin1024/vcrpy/blob/master/docs/advanced.rst)). Default matching is `method, scheme, host, port, path, query`, with no body ([configuration](https://github.com/kevin1024/vcrpy/blob/master/docs/configuration.rst)) |
| [pytest-recording](https://github.com/kiwicom/pytest-recording) | Python | 0.14.0 (2026-10-01) | vcrpy | `none` "to prevent unintentional network requests". Adds `rewrite` mode and `--block-network` | As vcrpy |
| [cassetter](https://github.com/Kludex/cassetter) | Rust core with Python, Node, Go and Rust SDKs sharing one format | Python 0.14.0 (2026-09-29); Rust crate 0.11.0 (first published 2026-09-10, 15 downloads when read) | Python patches clients; Rust wraps `reqwest` and `tonic` by explicit injection | `none`, `once`, `new_episodes`, `all`, `rewrite` ([Rust README](https://github.com/Kludex/cassetter/blob/main/crates/cassetter/README.md)) | Yes, at write time: `authorization`, `cookie`, `set-cookie`, `x-api-key`, `api-key`, `x-goog-api-key` and other headers; query parameters such as `api_key`; JSON fields such as `password` and `access_token`. Extra lists add to these. Binary protobuf is not scrubbed |
| [Polly.js](https://github.com/Netflix/pollyjs) | JS | 6.0.6 (2023-07-20); last push 2025-05-31 | Adapters | `replay`, with `recordIfMissing: true` | Not checked. Has `expiresIn` with `expiryStrategy` `warn` (default), `error` or `record` ([configuration](https://github.com/Netflix/pollyjs/blob/master/docs/configuration.md)) |
| [nock](https://github.com/nock/nock) | Node | 15.0.1 (2026-10-06); native `fetch` since [v14.0.0](https://github.com/nock/nock/releases/tag/v14.0.0) (2025-01-24) | Overrides `http.request` | Nock Back `dryrun`; also `wild`, `record`, `update`, `lockdown` | Not checked |
| [httpmock](https://docs.rs/httpmock/0.8.3/httpmock/struct.MockServer.html) | Rust | 0.8.3 (2026-02-04) | Local mock server; `record`/`playback` behind the `record` feature, `forward_to`/`proxy` behind `proxy` | Explicit | Request-header selection on record |
| [rvcr](https://github.com/ChorusOne/rvcr) | Rust | 0.2.1 (2024-03-05) | `reqwest-middleware` | Explicit | Not stated |
| `wiremock` 0.6.5, `mockito` 1.7.2 | Rust | 2025-08 / 2026-02 | Local mock servers | Stubs only. Neither README mentions recording | — |
| [Hoverfly](https://docs.hoverfly.io/en/latest/pages/keyconcepts/modes/modes.html) | Go proxy, any client | v1.12.15 (2026-09-21), Apache-2.0 | HTTP(S) proxy | Capture, simulate, spy, synthesize, modify, diff | Not checked |

Notes from the sources:

- cassetter's README lists its complaints about VCR.py: unsafe by default, an unsafe YAML loader, slowness, reliance on "undocumented internals that break on minor version bumps", and JSON stored as escaped strings. These are the author's claims ([README](https://github.com/Kludex/cassetter#why)).
- The Rust crate "buffers request and response bodies" and supports unary gRPC only ([Rust README](https://github.com/Kludex/cassetter/blob/main/crates/cassetter/README.md), "Current limits"). So a streamed answer is replayed in full, not chunk by chunk.
- Hoverfly's spy mode replays a match and "otherwise, the request will be passed through to the real API" ([spy](https://docs.hoverfly.io/en/latest/pages/keyconcepts/modes/spy.html)). Its diff mode "forwards a request to an external service and compares a response with currently stored simulation", then returns the differences ([diff](https://docs.hoverfly.io/en/latest/pages/keyconcepts/modes/diff.html)).

### 1.4 Redaction

- LangChain's docs warn: "VCR will by default record authentication headers and other sensitive information in cassettes." The project ships a three-header filter and asks authors to inspect each generated cassette before committing it ([chat_models.py](https://github.com/langchain-ai/langchain/blob/master/libs/standard-tests/langchain_tests/integration_tests/chat_models.py), `enable_vcr_tests`).
- cassetter filters at write time ("cassettes never contain secrets"), and custom lists add to the built-in ones rather than replacing them ([README](https://github.com/Kludex/cassetter#safe-by-default)).
- Pydantic AI's recording hooks strip `anthropic-*` headers. Its test guide therefore says that the live request, not the cassette, is the only place a test can check beta headers ([tests/AGENTS.md](https://github.com/pydantic/pydantic-ai/blob/main/tests/AGENTS.md)).

### 1.5 When recordings are refreshed

- Most projects record once and refresh by hand. The AI SDK copies a response from a run of an example script. Pydantic AI says: "Record cassettes with `--record-mode=rewrite`, verify playback without the flag, and review diffs" ([tests/AGENTS.md](https://github.com/pydantic/pydantic-ai/blob/main/tests/AGENTS.md)).
- Only Polly.js offers a time-based expiry (`expiresIn`), and by default it only warns ([configuration](https://github.com/Netflix/pollyjs/blob/master/docs/configuration.md)).
- Model retirement forces a refresh. Pydantic AI's `pyproject.toml` silences a deprecation warning for `claude-sonnet-4-5` because "Recordings across the suite were made against it". It notes that the model "reaches end of life on 2026-11-30", so "any re-recording after that date needs the move first" to a current model ([pyproject.toml](https://github.com/pydantic/pydantic-ai/blob/main/pyproject.toml)).
- Recording needs keys. Pydantic AI says: "Recording cassettes needs provider API keys and isn't trivial, so contributors routinely under-test the real behavior" ([tests/AGENTS.md](https://github.com/pydantic/pydantic-ai/blob/main/tests/AGENTS.md)).

### 1.6 Detecting provider changes

| Method | Who | What it catches | Source |
|---|---|---|---|
| Daily live integration tests (13:00 UTC) against the real APIs of the OpenAI, Anthropic, Fireworks, Groq, Mistral, xAI, Vertex, Gemini and AWS packages | LangChain | Behaviour and format changes | [integration_tests.yml](https://github.com/langchain-ai/langchain/blob/master/.github/workflows/integration_tests.yml) |
| Daily refresh of model metadata from models.dev, opening a PR | LangChain | New models and changed limits or capabilities | [refresh_model_profiles.yml](https://github.com/langchain-ai/langchain/blob/master/.github/workflows/refresh_model_profiles.yml) |
| Weekly live gateway model catalog check (Mondays), posting to Slack on failure | Pydantic AI | Models removed or renamed | [gateway-model-health.yml](https://github.com/pydantic/pydantic-ai/blob/main/.github/workflows/gateway-model-health.yml) |
| Canary on the newest SDK versions allowed by the declared ranges; blocks releases | Pydantic AI, added after `anthropic` 1.0.0 broke fresh installs | SDK changes | [latest-versions-canary.yml](https://github.com/pydantic/pydantic-ai/blob/main/.github/workflows/latest-versions-canary.yml) |
| An issue opened on a provider API change, and on model list diffs (new and obsolete IDs) | Vercel AI SDK (triggered by `repository_dispatch`) | API and model changes | [ai-provider-api-changes.yml](https://github.com/vercel/ai/blob/main/.github/workflows/ai-provider-api-changes.yml), [ai-provider-models.yml](https://github.com/vercel/ai/blob/main/.github/workflows/ai-provider-models.yml) |
| `system_fingerprint` in each Chat Completions response, which "represents the backend configuration" for use "in conjunction with the `seed`" | OpenAI | Server-side changes that affect determinism | [chat_completion.py](https://github.com/openai/openai-python/blob/main/src/openai/types/chat/chat_completion.py) |
| Diff mode: the live response compared with the stored one | Hoverfly | Any HTTP change | [diff](https://docs.hoverfly.io/en/latest/pages/keyconcepts/modes/diff.html) |

### Fit for Goodfetch

- **The seam already exists.** ADR 0012 has the engine make every model call behind its own provider interface. `STYLE.md` already justifies a mock adapter for a remote Model provider. A scripted Model provider that works like `ScriptedModel` would fail on an unexpected extra call and on unused steps. That suits the Agent loop, Code mode programs, Repair and Decision providers, because the test then fixes the exact sequence of model calls. The SDK-handler form of the Model provider Plugin kind (ADR 0033) would let the TypeScript and Python SDK suites script models the same way.
- **The Effect journal is already a semantic recording.** ADR 0010 journals model calls, and a resumed run reuses a journaled answer. A run replayed from its journal is deterministic at the engine's own boundary, before any HTTP. HTTP-level recordings remain useful for the three wire formats (OpenAI-compatible through `async-openai`, native Anthropic, Gemini). There the AI SDK's pattern applies: real responses as fixtures per format, served locally.
- **A proxy avoids patching clients.** The engine reaches providers by URL, and `async-openai` 0.42.2 exposes `with_api_base` and `with_http_client`. A replay proxy at a base URL therefore needs no client patching, which is the case vcrpy #1062 and Hoverfly make. The Tower layer on every HTTP attempt (ADR 0012) is a second place to record. cassetter's Rust wrapper is not a `reqwest::Client`, so it would not plug into `async-openai` directly. This is an inference from its README, not a test.
- **Lessons from the projects.** Match on the request body, or capture what is sent; vcrpy's defaults do neither. Fail on unused recordings (Pydantic AI). Make PR runs playback-only with a fake key (LangChain, pytest-recording). Scrub secrets at write time by default (cassetter). Keep a list of which recordings depend on which model ID, so a retirement date can be seen in advance.

## 2. Tests against evaluations

### 2.1 What a gating evaluation is made of

- **pass@k and pass^k.** pass@k is "the chance that at least one out of k i.i.d. task trials is successful": 1 − E[C(n−c,k)/C(n,k)], from the Codex paper ([Chen et al., 2021](https://arxiv.org/abs/2107.03374)). τ-bench added pass^k, "the chance that all k i.i.d. task trials are successful, averaged across tasks": E[C(c,k)/C(n,k)]. τ-bench grades by comparing the final database state with the goal state, runs at least 3 trials per task, and reports pass^8 below 25% in retail for gpt-4o ([Yao et al., 2024-06-17](https://arxiv.org/abs/2406.12045)).
- **Error bars.** Miller ([arXiv 2411.00640](https://arxiv.org/abs/2411.00640), 2024-11-01, Anthropic) makes five recommendations. Compute the standard error of the mean with the Central Limit Theorem; for 0/1 scores this is √(p(1−p)/n). Use clustered standard errors when questions come in related groups. Reduce variance by resampling answers. Compare models on question-level paired differences. Use power analysis. The worked example needs about 969 questions to detect a 3-point difference at α = 0.05 and 80% power.
- **Tool support.** Inspect AI (0.3.279, 2026-10-13, UK AISI) runs `epochs` with reducers `mean`, `mode`, `majority`, `pass_at_{k}`, `pass_k_{k}` (that is, pass^k) and `at_least_{k}`. It provides metrics `stderr`, `bootstrap_stderr`, `ci` (Student-t or cluster bootstrap) and `ci_wilson` for 0/1 scores. A `cascade()` scorer runs cheap scorers first and calls a grader model only when they don't settle a sample ([metrics.qmd](https://github.com/UKGovernmentBEIS/inspect_ai/blob/main/docs/metrics.qmd), [multiple-scorers.qmd](https://github.com/UKGovernmentBEIS/inspect_ai/blob/main/docs/multiple-scorers.qmd)). Pydantic Evals offers `repeat=n` with per-case grouping ([multi-run.md](https://github.com/pydantic/pydantic-ai/blob/main/docs/evals/how-to/multi-run.md)).
- **Anthropic's guidance**, "Demystifying evals for AI agents" ([2026-01-09](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents)):
  - Capability evaluations "start at a low pass rate", and those "with high pass rates can 'graduate' to become a regression suite".
  - Code-based graders are "Fast", "Cheap", "Objective" and "Reproducible". Model-based graders are "Non-deterministic" and need "calibration with human graders".
  - "It's often better to grade what the agent produced, not the path it took." Each trial should start "from a clean environment".
  - "You won't know if your graders are working well unless you read the transcripts."
  - "20-50 simple tasks drawn from real failures is a great start."
  - Its example: a 75% per-trial success rate gives about 42% pass^3.
- **LLM judges agree with people only partly.** Online-Mind2Web's WebJudge reaches "around 85% agreement with human judgment". The same paper finds WebVoyager's automatic judge has "low agreement with human judgment", and that Browser Use scored 30% when checked by people ([arXiv 2504.01382](https://arxiv.org/abs/2504.01382)).

Computed here, not taken from a source:

| Observed passes | 95% Wilson interval |
|---|---|
| 18/20 | 0.70–0.97 |
| 45/50 | 0.79–0.96 |
| 180/200 | 0.85–0.93 |

Zero failures in n trials bounds the true failure rate below about 3/n at 95% confidence: 60 trials for 5%, 300 for 1%. pass^k falls fast: a 95% per-trial rate gives 0.86 at k = 3 and 0.66 at k = 8.

### 2.2 How competitors run AI behaviour in CI

| Project | On every PR | Live model in CI | Who grades | Gate | Source |
|---|---|---|---|---|---|
| Stagehand 3.7.3 | Unit tests with stubs; browser tests | `evals/regression` job, internal PRs only, skipped with the `skip-evals` label. Runs on Browserbase with `EVAL_TRIAL_COUNT: "3"`, concurrency 25, and Anthropic, OpenAI and Google keys | Task code returns `_success`. Bench tasks use an LLM judge plus deterministic checks; "gated" means "the judge passed it, a deterministic check … failed it" | Fails if the Braintrust primary score is below 0.8 | [ci.yml](https://github.com/browserbase/stagehand/blob/main/.github/workflows/ci.yml), [evals README](https://github.com/browserbase/stagehand/blob/main/packages/evals/README.md) |
| Browser Use 0.13.11 | Unit tests with mock LLM | `evaluate-tasks` runs YAML tasks on live sites (for example "Go to amazon.com, search for 'laptop'"). `eval-on-pr.yml` triggers an external platform (`InteractionTasks_v8`) for owners, members and collaborators | LLM judge (`gemini-3.1-flash-lite`) against `judge_context` criteria | Fails only at a 0% pass rate; otherwise posts a PR comment | [test.yaml](https://github.com/browser-use/browser-use/blob/main/.github/workflows/test.yaml), [evaluate_tasks.py](https://github.com/browser-use/browser-use/blob/main/tests/ci/evaluate_tasks.py), [eval-on-pr.yml](https://github.com/browser-use/browser-use/blob/main/.github/workflows/eval-on-pr.yml) |
| Skyvern 1.0.55 | pytest with dummy keys | None. WebVoyager runs are published under `evaluation/results` | — | — | [ci.yml](https://github.com/Skyvern-AI/skyvern/blob/main/.github/workflows/ci.yml), [evaluation/](https://github.com/Skyvern-AI/skyvern/tree/main/evaluation) |
| LangChain | Fakes plus VCR playback | Daily scheduled integration tests | Test assertions | Pass/fail | §1.6 |
| OpenAI Agents SDK 0.23.1 | Scripted models | None | — | — | §1.2 |
| Anthropic and OpenAI Python SDKs | Mock server from the OpenAPI spec | None | — | — | §1.2 |
| Vercel AI SDK 7 | Mocks plus real-response fixtures | None | — | — | §1.2 |
| Pydantic AI 2.55.0 | Cassettes, `TestModel` | Weekly live gateway catalog check. Live sandbox-provider tests run only nightly or on a maintainer's dispatch, because "Every test starts a billed sandbox" ([sandbox-live.yml](https://github.com/pydantic/pydantic-ai/blob/main/.github/workflows/sandbox-live.yml)) | Test assertions | Pass/fail | §1.6 |

The Stagehand evals README adds details on retries and failure handling:

- When a provider returns 429, "that provider's width is halved for 60 s and the row is retried once", and the row is tagged `provider_throttled`.
- "When the judge grades nothing (a retired model, a bad key), every row fails closed … and exits 1."
- `--preview` prints the plan "before paying for a run".
- An "observation task recorder" captures a rendered page for static `extract()` and `observe()` regressions.

([evals README](https://github.com/browserbase/stagehand/blob/main/packages/evals/README.md))

### 2.3 Cost control

- **Limit live jobs to trusted PRs, schedules or labels.** Stagehand runs on internal PRs, with a skip label. Browser Use triggers its platform only for owners, members and collaborators. LangChain's live runs are daily, not per PR.
- **Use batch pricing.** OpenAI's Batch API gives a "50% cost discount compared to synchronous APIs". Each batch completes within 24 hours, and "Running evaluations" is a listed use ([batch guide](https://developers.openai.com/api/docs/guides/batch)). Anthropic's Message Batches API has "most batches finishing in less than 1 hour while reducing costs by 50%" ([batch processing](https://platform.claude.com/docs/en/build-with-claude/batch-processing)).
- **Grade cheaply first and size the sample.** Inspect's `cascade()` calls the grader model only when cheaper scorers did not settle a sample. Miller's power analysis gives the number of questions needed before spending anything.

### Fit for Goodfetch

- **Grading is already settled.** ADR 0013 forbids a model alone from judging success. ADR 0025 says "the oracle decides", and the Proving ground's expected results come from the seed, never from a tool. So Goodfetch's evaluations can use code-based grading throughout. Stagehand and Browser Use rely on LLM judges, and Online-Mind2Web measures how far such judges stray.
- **pass^k is the reliability measure ADR 0025 already implies.** That ADR asks for zero false successes. The 3/n bound shows what "zero" can claim at a given trial count: about 300 clean trials to say "below 1%".
- **The statistics match.** ADR 0025's statistics (claims declared beforehand, Δ fixed per decision, block-bootstrap intervals, the page or task as the pairing unit) agree with Miller's recommendations.
- **Live evaluations are measurements, not tests.** Kept outside `cargo-nextest` and run on a schedule or a label, they don't conflict with `STYLE.md`'s rule to quarantine a flaky test.
- **Most AI cases replay without a model.** Plans replay with no model call (ADR 0009). Only first compilation, Repair, the Agent loop, per-page extraction and Decision provider calibration need live models.
- **Batch pricing fits single-call cases only.** Single-call evaluation cases, such as extraction to a schema and Decision provider calibration, can use batch pricing. Multi-turn Agent loops cannot, because each turn depends on the last. This is an inference.

## 3. Limits of determinism

| Model provider | Control | What the provider documents | Source |
|---|---|---|---|
| OpenAI, Chat Completions | `seed` | "This feature is in Beta … best effort … Determinism is not guaranteed"; watch `system_fingerprint` | [completion_create_params.py](https://github.com/openai/openai-python/blob/main/src/openai/types/chat/completion_create_params.py) (openai-python 3.29.0) |
| OpenAI, Responses API | — | No `seed` parameter in the request type | [response_create_params.py](https://github.com/openai/openai-python/blob/main/src/openai/types/responses/response_create_params.py) |
| OpenAI, current models | `temperature`, `top_p` | "When reasoning effort is not `none`, remove `temperature`, `top_p`, and `top_logprobs`" (page covering the GPT-6 family) | [latest-model guide](https://developers.openai.com/api/docs/guides/latest-model) |
| Anthropic, Claude Opus 4.7 and later | `temperature`, `top_p`, `top_k` | A non-default value "returns a 400 error". "If you were using `temperature = 0` for determinism, note that it never guaranteed identical outputs on prior models." The Python SDK v1.0 (2026-08-20) removed the parameters | [Opus 5.5 migration guide](https://platform.claude.com/docs/en/models/opus-5-5/migration-guide), [SDK MIGRATION.md](https://github.com/anthropics/anthropic-sdk-python/blob/main/MIGRATION.md) |
| Gemini | `temperature`, `top_p`, `top_k` | Changelog, 2026-07-21: "now deprecated". From Gemini 3.6 Flash and 3.5 Flash-Lite, "The API ignores these parameters", and "In future model generations, supplying these parameters returns an HTTP 400 error". Earlier Gemini 3 guide: keep temperature at 1.0, because lower values "may lead to … looping or degraded performance" | [changelog](https://ai.google.dev/gemini-api/docs/changelog), [Gemini 3.6](https://ai.google.dev/gemini-api/docs/whats-new-gemini-3.6), [Gemini 3](https://ai.google.dev/gemini-api/docs/gemini-3) |
| Gemini | `seed` | "the model makes a best effort to provide the same response … By default, a random number is used" | [google-genai types.py](https://github.com/googleapis/python-genai/blob/main/google/genai/types.py) (2.30.0) |
| vLLM (v0.31.0) | `seed` (default 0), `VLLM_BATCH_INVARIANT=1` | "vLLM does not guarantee the reproducibility of the results by default." Online, "you can only enable batch invariance". Reproducible only "on the same hardware and the same vLLM version". Batch invariance is in beta and needs NVIDIA compute capability 8.0+, AMD Instinct or Intel XPU | [reproducibility.md](https://github.com/vllm-project/vllm/blob/main/docs/usage/reproducibility.md), [batch_invariance.md](https://github.com/vllm-project/vllm/blob/main/docs/features/batch_invariance.md) |
| llama.cpp server (build b11583) | `seed` (default −1, random), `cache_prompt` (default on) | Because "the logits are **not** guaranteed to be bit-for-bit identical for different batch sizes", enabling `cache_prompt` "can cause nondeterministic results" | [server README](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md) |
| Ollama (v0.40.3) | `seed` (default 0), `temperature` (default 0.8) | The docs say a fixed seed "will make the model generate the same text for the same prompt" | [modelfile.mdx](https://github.com/ollama/ollama/blob/main/docs/modelfile.mdx) |

Why outputs vary:

- **Server load.** Thinking Machines (Horace He, 2025-09-10) traces most endpoint nondeterminism to server load: "the load (and thus batch-size) nondeterministically varies". At temperature 0, Qwen3-235B produced 80 distinct completions out of 1,000. With batch-invariant kernels, all 1,000 were identical. The cost was 26 s for 1,000 requests by default against 42–55 s deterministic, on one GPU running Qwen3-8B ([blog](https://thinkingmachines.ai/blog/defeating-nondeterminism-in-llm-inference/)).
- **Measured spread.** Atil et al. ran five models set to be deterministic, 10 times each. They found "accuracy variations up to 15%" and a best-to-worst gap "up to 70%" ([arXiv 2408.04667](https://arxiv.org/abs/2408.04667)).
- **Ollama in practice.** Open issues report different outputs with a fixed seed: [#12559](https://github.com/ollama/ollama/issues/12559) (structured output, 2025-10-10) and [#5321](https://github.com/ollama/ollama/issues/5321) (first run differs, 22 comments). [#17744](https://github.com/ollama/ollama/issues/17744) (2026-08-14) says a Modelfile's `temperature 0` is ignored on `/v1/chat/completions` unless the request sets `temperature`.

### Fit for Goodfetch

- **No deterministic test can rest on a cloud model.** Current Anthropic and Gemini models refuse or ignore the sampling parameters, and OpenAI's seed is best-effort and Chat-Completions-only. Determinism in the suites has to come from scripted or recorded models, the Effect journal, and Plan replay. Live runs are measurements.
- **Local models can be reproducible.** Only vLLM documents a way: batch invariance, on fixed hardware and a fixed version. For llama.cpp the documented levers are a fixed `seed` and `cache_prompt` off. Pinning a single slot (`-np 1`) is an inference from the batch-size note, not a documented guarantee.
- **Send sampling parameters on every local request.** ADR 0012 sends local models through the OpenAI-compatible path, where Ollama #17744 bites. The engine should send any sampling parameters it relies on in each request, and record the server version and model digest with each evaluation run.

## 4. User pains

### Flaky AI tests

- Tests that expect specific model output fail at random. spiceai [#6399](https://github.com/spiceai/spiceai/issues/6399) (2025-07-07, open): "Some tests are structured to be dependent on model outputting an expected results". effectful [#773](https://github.com/BasisResearch/effectful/issues/773) (2026-09-02, open): a weak model "often exceeds retry limits on basic examples, leading to spurious CI failures".
- Seeds disappoint users. The OpenAI forum thread "[Seed param and reproducible output do not work](https://community.openai.com/t/seed-param-and-reproducible-output-do-not-work/487245)" opened 2023-11-09 and has more than 20 replies. The Ollama issues are listed in §3.
- Record-and-replay breaks when clients change.
  - vcrpy [#927](https://github.com/kevin1024/vcrpy/issues/927) (2025-07-03): a change in `google-genai`'s stream reading "results in an infinite loop when using VCR".
  - vcrpy [#989](https://github.com/kevin1024/vcrpy/issues/989) asks whether VCR works with streamable HTTP and SSE.
  - vcrpy [#1062](https://github.com/kevin1024/vcrpy/issues/1062) (2026-10-08) proposes a local server mode because "the patching layer is where most breakages come from".
  - Polly.js [#488](https://github.com/Netflix/pollyjs/issues/488), "Node's native fetch support", has been open since 2023-10-12.
  - Anthropic's v1 migration guide says that respx, pytest-httpx and vcrpy "patch the `httpx` package, which the SDK no longer uses. These libraries can silently fail" ([MIGRATION.md](https://github.com/anthropics/anthropic-sdk-python/blob/main/MIGRATION.md)).

### Evaluation suites that rot

- SWE-bench: 68.3% of samples were filtered out for underspecified problems (38.3%), unfair unit tests (61.1%) or other issues, leaving 500 in SWE-bench Verified ([OpenAI, 2024-08-13](https://openai.com/index/introducing-swe-bench-verified/)).
- WebVoyager: "a simple agent that primarily uses Google Search can already solve up to 51% of the tasks", and coverage "is limited to just 15 websites" ([Online-Mind2Web](https://arxiv.org/abs/2504.01382)). Skyvern keeps a separate file of 8 outdated WebVoyager tasks, such as Apple Vision Pro release dates, next to its file of 635 tasks ([datasets](https://github.com/Skyvern-AI/skyvern/tree/main/evaluation/datasets)).
- Retired models break recordings and judges: Pydantic AI's end-of-life note (§1.5) and Stagehand's dead-judge rule (§2.2).
- Saturation: "Eval saturation occurs when an agent passes all of the solvable tasks" ([Anthropic](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents)).

### Cost

- Live evaluations are reserved for maintainers or run on a schedule (Stagehand, Browser Use, LangChain; §2.2–2.3), and contributors under-test because recording needs keys (Pydantic AI; §1.5).
- Stagehand caps concurrency per provider and halves it after a 429 ([evals README](https://github.com/browserbase/stagehand/blob/main/packages/evals/README.md)).

### Fit for Goodfetch

- **Assert outcomes, never model wording.** A test checks the Success check result, Grounding, the Plan file, the Effect journal or the Record. That is the opposite of the spiceai pattern.
- **Avoid the main rot sources.** The Proving ground is generated from a seed with known expected results (ADR 0025), so it escapes the time-sensitivity and shortcut problems of live-site sets like WebVoyager. Two rot sources remain: model retirement (recordings and pinned models) and saturation (tasks that every model passes).
- **The replay proxy should be language-neutral.** Contributors to the SDKs and to Recipes face the same "needs keys" barrier. A proxy at the HTTP level with write-time scrubbing avoids the client-patching failures above.

## Unverified

- What sends the Vercel AI SDK's `repository_dispatch` events, that is, what watches provider releases and model lists. It is not in the repository.
- The value of Stagehand's `EVAL_MODELS` repository variable, and what Browser Use's external evaluation platform (`InteractionTasks_v8`) runs or gates. Both are private.
- Which Gemini models beyond 3.6 Flash and 3.5 Flash-Lite ignore the sampling parameters today. Secondary reports also name "3.5 Flash Cyber", but the primary page fetched did not list it.
- Whether OpenAI's `seed` has any effect on GPT-6 family models. The pages read do not say.
- Whether Ollama v0.40.3 fixed the seed issues #12559 and #5321. Both were still open on 2026-10-13.
- Polly.js has had no release since 2023-07-20. No official deprecation notice was found.
- cassetter's Rust crate is young: 15 downloads, first published 2026-09-10. Its criticisms of VCR.py are the author's own. Fitting it to `async-openai` was not tried.
- The figure of about 1,720 Pydantic AI cassettes counts YAML files under `cassettes` folders in the repository tree on 2026-10-13. It may include non-HTTP cassettes.
- Thinking Machines' timings come from one GPU and one model. vLLM's batch-invariant performance on Goodfetch's reference host is unmeasured.
- The Wilson intervals, the 3/n bound and the pass^k figures in §2.1 were computed for this note, not taken from a source.
