## AI features and agent skills

Decided in [ADR 0065](../adr/0065-ai-features-are-tested-without-live-models-and-measured-by-oracle-graded-evaluations.md). Suites are the Quick, Pre-merge and Release suites of [ADR 0061](../adr/0061-three-test-suites-each-run-by-one-just-command-under-a-time-budget.md); Live checks are those of [ADR 0064](../adr/0064-tests-are-never-retried-and-unpinned-checks-run-as-live-checks.md), in no suite. A **live model** is one a provider serves and Goodfetch doesn't pin (a cloud model, directly or through a gateway); a pinned local model on a pinned server is a test input.

### Stand-ins for a model

| Layer | Stand-in | Suite |
|---|---|---|
| In-process AI logic | the Scripted model | Quick |
| Each adapter's wire format | Model recordings replayed by `httpmock` at the adapter's base URL | Pre-merge |
| The binary, Journeys, AI end to end | the Proving ground's Fake model, in all three formats | Pre-merge |
| SDK code that calls a model | a model callback registered through the SDK handler | Pre-merge |
| Local model servers | the real server, pinned, with a pinned small model | Release (Linux cell) |

- **No test of an AI feature calls a live model.** Only the Live checks below reach one.
  _Why:_ providers no longer offer determinism: current Anthropic models refuse `temperature`, Gemini ignores it, and OpenAI's `seed` is best effort.
- **The Scripted model fails on a call beyond its script and on unused steps.** Script errors and faults as replies, and assert on the requests it recorded.
- **AI Journeys name the Fake model as their Model provider.** Ask it for faults (rate limit, overload, timeout, cut stream, malformed JSON, refusal, wrong but plausible answer) through the task, never through a test-only switch in the binary.
- **Check Usage against the Fake model's own token count.**

### What an AI test asserts

- **Assert outcomes Goodfetch owns:** the Success check result, Grounding, the Plan file, the Effect journal, the Record, the Error code, Usage, what the oracle saw. Never a model's wording.
- **Never rely on temperature or a seed for determinism.**
- **Snapshot the whole request each AI operation sends, per format, with `insta`** (Quick). A prompt or tool-description change then shows as a reviewed diff.

### Model recordings

- **Record each cell of the matrix:**
  - per format on the direct provider: text, tool call and result turn, parallel tool calls, structured output, streaming of each, usage (cached and reasoning tokens), prompt caching, search grounding, image input, and the errors the engine branches on;
  - the named gateways, for the OpenAI-compatible format;
  - each hosted Decision provider (Jev, OpenAI's Decisions API): a choice, an abstention, a low-confidence choice, and its errors.

  A missing cell fails a Quick check. While there is no revenue ([ADR 0076](../adr/0076-tests-and-ci-run-on-free-tiers-and-ai-calls-go-through-the-vercel-ai-gateway.md)), cells are recorded through the Vercel AI Gateway and labelled `via gateway`, which counts; the direct-provider, other-gateway and hosted Decision provider cells, and Gemini's native format, which the gateway does not serve, wait and are reported missing without failing.
- **Match on method, path and the canonical JSON body.** An unmatched request fails, and so does an unused recording.
  _Why:_ matching on the path alone replays a stale answer to a changed request.
- **Replay each stream whole and re-cut at seeded boundaries.** The stream parsers also have `proptest` and a fuzz target.
- **Keep only allowlisted headers at write time:** `content-type`, the version and beta headers the adapter sends, `retry-after` and the rate-limit headers the adapters read. Remove key query parameters, and replace request and organization IDs. A Quick check fails on anything else, or on anything shaped like a provider key.
- **Record only against Proving ground pages and tasks.**
- **Refresh on cause:** a changed request, a provider-check report, or 30 days before a manifest model's retirement. Use the recorder job, which opens a pull request, and review the diff. Never refresh on a timer or automatically.
- **Write faults that aren't worth recording as explicit `httpmock` stubs**, marked as synthetic.

### Pinned local models and the in-engine model

- **llama.cpp server and Ollama run in the Release suite's Linux cell**, each at a pinned version with a pinned small model downloaded by digest before ADR 0043's deny, with the dependency installs. vLLM joins them on its CPU build where it can serve the model.
  - Assert protocol behaviour only.
  - Send every sampling parameter the engine relies on.
  - Record the server version and model digest.
- **In the release pipeline**, every model on ADR 0012's tested list runs the same tests. vLLM runs there on its CPU build; a GPU host is declined while there is no revenue (ADR 0076), so where the CPU build can't serve the model, vLLM waits. LM Studio runs there where a runner supports its headless server.
- **The in-engine Decision provider model is checked on a labelled set with a margin:** on Linux in the Pre-merge suite, and on every OS in the Release suite. Never compare confidences exactly.

### Live checks that reach live models

- **The provider check runs nightly and compares shapes, never content.** It covers fields, types, stream events, error bodies and the headers adapters read, for every recorded cell: direct providers, gateways and hosted Decision providers (while there is no revenue, only the cells recorded `via gateway`; ADR 0076). A difference opens or updates its `live-check` issue.
- **Every live model call goes through the Vercel AI Gateway on its free tier** ([ADR 0076](../adr/0076-tests-and-ci-run-on-free-tiers-and-ai-calls-go-through-the-vercel-ai-gateway.md)): one key, `AI_GATEWAY_API_KEY`, free-tier models only, no credits bought and auto top-up off, so no job can spend money. A `429` or an exhausted free credit makes the run inconclusive, never a purchase.
- **Every live job stops itself at its per-run cap.** The engine's runs use a currency Limit, Claude Code uses `maxBudgetUsd`, Codex has no budget setting of its own and calls models only through the gateway key, whose free credit caps it since the key cannot spend, and the provider check and recorder compute their worst case before starting. Each job prints its worst case first.
- **Coding agents in live jobs make no other calls:** Claude Code runs with `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`, and Codex with its update check and analytics turned off.
  _Why:_ the job reaches only its named model hosts, and ADR 0043's deny refuses anything else.
- **Live jobs run on the gateway's free tier from M4.** Fork pull requests never receive model keys.

### Evaluations

- **Evaluations are `bench/` campaigns on the local Proving ground**, never tests. They come in three sizes: pull-request (only for changes to prompts, request snapshots, catalog descriptions or skills), nightly (plus a weekly run on larger models), and ADR 0025's release benchmark.
- **The oracle grades every trial.** No model grades an Evaluation. Record the engine's Success check and Grounding results beside the oracle's verdict.
- **Run 3 trials per task and model** at the pull-request and nightly sizes, each from a clean Goodfetch home. Every trial counts.
- **Report** the verified rate with its 95% Wilson interval, pass^3, false successes, safety violations, tokens and dollars per verified result, and the Success check and Grounding disagreements with the oracle. State the bound that "zero" can claim at the trial count (about 3/*n*).
- **A false success or a safety violation fails an Evaluation at any size** and blocks the next release candidate as a correctness failure.
- **The regression rule** is the mean over tasks of the paired difference in verified rate against the last accepted baseline on the same models. The difference is in percentage points (never a log ratio), with a block-bootstrap interval. It fails only when the interval's upper bound is below −Δ. An inconclusive result is reported, never a failure.
- **Δ is proposed at 10 points and waits for the owner's approval.** Until then, a regression opens or updates a `live-check` issue and blocks no release candidate.
- **A task joins its regression set at pass^3 ≥ 0.9 on every nightly model for five nights.** Add harder tasks once a regression set passes everything for 30 nights. Add a task for every AI bug fixed.
- **The pull-request size uses development seeds; nightly and release use held-out seeds.**
  _Why:_ prompts tuned on the tasks that judge them overfit.
- **Pin models in the manifest, chosen from the gateway's free tier, and run old and new side by side for five nights before dropping one.** An Evaluation or skill Evaluation whose model is not in the free tier waits, reported inconclusive, until revenue (ADR 0076). Record the model, any fingerprint the provider returns, the gateway, the local server's version and digest, the commit, the seed and the prompt snapshot hashes.

### Agent skills and tool descriptions

- **Skills are checked with no model:**
  - in Quick, `xtask codegen --check`, which renders them for each Product and fails a skill that breaks the Agent Skills frontmatter limits, names an operation its Product's agents cannot call or a command its executable lacks, presents a person-only answer as the agent's action, or uses a word of the wording list;
  - in Quick too, each executable's unit test that every option its skills name, in an example or a code span, is an option of the command they name (`goodfetch_test_support::unknown_options` over the clap command line);
  - in Pre-merge, a run of every example command against the local Proving ground, checked by the oracle.
- **Each Product's skills ship as a Claude Code plugin, and its `plugin` test runs `claude plugin validate --strict`** in Pre-merge, at the version `test-support/claude-code/pins.toml` pins with a SHA-256 per platform. `ci/claude-code-fetch.sh`, part of `just fetch`, downloads that release, never through npm; a CI job fetches it only when it sets `GOODFETCH_FETCH_CLAUDE_CODE=yes`, as every job that runs the plugin tests must. The test runs it offline: autoupdate, telemetry, error reporting and nonessential traffic are off, every proxy variable points at a local listener, and a connection to it fails the test.
- **Skill Evaluations run the CLI with and without the skills**, through promptfoo, on Claude Code (small, mid and large models) and Codex. The oracle grades; `skill-used` is a signal. Open an issue on any skill that doesn't beat its baseline on some model.
- **Snapshot the catalog's MCP projection per Product and profile** (Quick). Keep each description at most 2,048 characters, and record each profile's total tool-definition size in the snapshot.
- **The tool-selection Evaluation checks a model's first tool call without executing it.**

### Manual tests

- **After a confirmed Evaluation regression, read the failed trials' Run traces** before accepting or fixing it.
  _Why:_ only a person can tell a wrong grade or a new failure mode from a real regression.
- **LM Studio**, where no CI runner can host its headless server: run the tested-list cases by hand before each release.
