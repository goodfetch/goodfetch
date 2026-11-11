# What coding agents misread is reshaped: an AI result holds its answer in how it ended, resource types are blocked by a setting, and a start names its `runId`

The [Agent-writability check](https://github.com/goodfetch/goodfetch/issues/484) gave fresh coding agents (Claude Sonnet and Haiku) each golden scenario's task statement and the sketch's stubs at `eb1e35b`, and nothing else. They wrote 72 programs that type-check, 42 of them correct, and beat the same models on the leaders' APIs (Sonnet 14 against 9, Haiku 13 against 6 on the 18 shared tasks). They also got six shapes wrong more than once ([findings](https://github.com/goodfetch/goodfetch/blob/8b7d2765/sdk-api-sketch/writability/README.md#findings-for-485)). ADR 0077's tie-breaker T makes such evidence decide between shapes that still tie, and G1, G3 and S2 decide where a misread shape hides a failure. [Close the pain coverage matrix](https://github.com/goodfetch/goodfetch/issues/485) decides each finding here: five change the SDK API (findings 2, 3, 5, 6 and 7), finding 4's proposal is declined because two languages cannot express it, and every misread, finding 1's included, gets a doc comment whose first line states the rule the agents missed. Decided on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461), by the agent on the owner's delegation.

## Decision

### An AI result holds its answer in how it ended (finding 2)

Five programs read an Agent's answer from `output` alone, so a task stopped by its step cap or a Limit lost the Grounded `partial` that scenario 11 asks for (G1). TypeScript and Rust can make the stopped case impossible to skip, and Python's `match` shows it, if the answer sits inside the value that says how the work ended.

- **`ended` carries the answer** (G1, S1, T). ADR 0079's `AiWorkResult<T>`, and so ADR 0100's `AgentResult<T>`, no longer has top-level `output` and `partial`. `ended` is one of two:
  - **`finished`**, with `output`, a complete `T`, validated and held to Grounding as ADR 0100 says;
  - **`stopped`**, with its `reason` (ADR 0079's and ADR 0100's stop reasons) and `partial`, what was gathered and Grounded, typed as partial.
- **A model that gives up stops the work** (G1). ADR 0100 counted the model's `gave_up` as `finished` with no output. It is now `stopped` with the reason `gave_up`, carrying `partial`; `reported` still holds the claim `gave_up` and its message. So `finished` always holds a complete `T`, and every early end is read in one place.
- **Per language** (S1, S4):
  - TypeScript: `ended` is `{ kind: "finished"; output: T } | { kind: "stopped"; reason: StopReason; partial: PartialOf<T> }`, narrowed on `ended.kind`.
  - Python: `Finished[T]` with `output` and `Stopped[T]` with `reason` and `partial`, matched by class (`case Finished(output=…)`).
  - Rust: `enum Ended<T> { Finished { output: T }, Stopped { reason: StopReason, partial: PartialOutput } }`. It is exhaustive: a third way to end is a new meaning, not a new value, so a newer engine cannot send one, and a `match` needs no wildcard arm. `StopReason` stays open (ADR 0079).
- **Unchanged:** `verdict`, `reported`, `requireVerified()` (which returns the output only when `verified`, so only from `finished`), `steps`, `usage`, `page`, and the error path, where `agentTask` carries the partial answer as before.

### Whole resource types are blocked by a setting (finding 3)

Three programs blocked images and fonts in the wrong place: a route set in the handler, after the page had loaded, or a before-navigation Hook scoped to too few Labels. ADR 0047 already decided the shape that cannot be misplaced: whole types are a setting, and finer blocking is a Hook.

- **The key is `browser.block_resources`** (S2, S3, G3). ADR 0047's `block_resources` takes ADR 0083's area-first spelling under the `browser.*` Session settings: a list of resource types, only `image`, `media`, `font` and `stylesheet` (a closed type in every language, any other refused with `setting_invalid` as G3 asks of inputs), empty by default, set where a Session opens or on any scope above it, a crawl's Run included. The engine's interception applies it to every page of the Session before any navigation, so it covers each browser fetch of a crawl, a sample comparison's included, and no program code has to run in time. It follows ADR 0083's rules for `browser.*` keys, and a Label refuses it with `setting_not_applicable`, as it refuses every `browser.*` key, since its Sessions take the Run's identity (ADR 0093). ADR 0047's warning in `standard` and `stealth` stands.
- **Golden scenario 06 uses it** in place of its before-navigation Hook, in all three languages. Hooks and ADR 0111's Rust route stay shown by the `hooks` snippets.
- **`CrawlContext.page` says it is already navigated** (G6): its doc comment states that the request's navigation has happened when the handler runs, so a route set there applies from the next navigation, and points to `browser.block_resources` and a before-navigation Hook.

### A start names its Run with `runId`, and failures are read as they stand (finding 6)

Two programs treated a fixed Run ID as a label and met a Run that had already ended; two more re-admitted failures on every start, or read the outcome log as current state.

- **The start option `id` is `runId`** (`run_id` in Python and Rust; S3, T), as Temporal's start takes `workflowId` with an ID reuse rule. Its doc comment's first line states ADR 0102's rule: an existing Run ID resumes the Run if it was interrupted, or returns it as it ended. ADR 0099 already spells the word so. `run.id` keeps its name on the value, where it is unambiguous, and `openRun` and `openCrawlRun` take the Run ID.
- **`run.failedRequests({ blocked? })`** (`failed_requests` in Python and Rust; G1, S2) reads, from the Run's committed state at the call, the crawl requests whose latest outcome is `failed`, and with `blocked: true` those whose latest is `blocked`, each as that `CrawlRequestOutcome`: exactly what `retryFailed` with the same option would re-admit. The sequence ends after what is committed. It is on a crawl's Run and on `CrawlRunView`, and like `outcomes()` it never slows the Run.
- **`outcomes()`'s doc comment opens with what it is:** every ending in commit order, a request ending again after `retryFailed`; a log, not current state.
- **The term Run ID joins `GLOSSARY.md`,** with "label" among its _Avoid_ terms, since a crawl's Label is another thing.

### The testing kit scripts the data an extraction should give (finding 5)

Two tests scripted the extracted data as a model reply, while `extract.mode` defaults to `compiled` and a compile request expects locators. ADR 0105 left compiled steps to Plan replay, which a test without a Plan cannot use.

- **`scriptedExtraction(data)`** (`scripted_extraction` in Python, `ScriptedReply::Extraction` in Rust; S2, T) is a Scripted model reply that answers one `extract` call in its active mode (ADR 0091's `extract.mode`): under `model`, with `data`; under `compiled`, the default, with the locators that read `data` from the page, which the testing kit writes by finding each value on the page, as Grounding locates one. The compiled step then reads the page with them as it would in production.
  > Amended by [ADR 0123](0123-a-typed-value-read-from-page-text-comes-from-an-engine-transform-under-a-stated-locale-and-time-zone.md): the kit answers in the model's answer form. For each value of `data` it finds the page text from which a Value transform under the locale the compile would use (`extract.locale` when set) gives that value ("01.02.2026" for `2026-02-01`), and answers with that text and transform under `model`, or with the locators and transform that read it under `compiled`.
- **A value not on the page** gets no locator, so the step raises ADR 0091's `extraction_incomplete`, naming the field, as a real compile would (G1). No new Error code.
- The Scripted model's other replies, and its `scripted_model_exhausted` and `scripted_model_unused`, are unchanged.

### Smaller shapes (finding 7)

- **An unpriced Run model is refused when the Run starts** (G6, G5). When a currency Limit or `ai.allowance.price` applies, a Run whose `ai.model` the price table cannot price fails its start with `model_price_unknown`, unless `plan.frozen` forbids every model call. A model given to one call with `model` is still refused at that call (ADR 0101).
- **Read-only results are covariant** (G7, S1). Python's result generics (`StepResult`, `AIWorkResult`, `AgentResult`, `Finished`, `Stopped`) declare their type parameter covariant, as `Sequence` does, so a `StepResult[Author]` is a `StepResult[object]`. TypeScript's `readonly` results already are, and Rust has no variance to declare.
- **A Session source's and a secret binding's `domains` are registrable domains** (G3, G5). The name stays, one form with ADRs 0056, 0092 and the `*_domains` settings (S1). A host that is not a registrable domain (`www.example.com`) is refused with `setting_invalid`, whose `fix` names the registrable domain, never reduced to it in silence, since reducing would widen what the source or the binding covers. The doc comment says so.
- **Each package's entry file opens with a map** (T): which package holds `Collie`, `Page`, `Run` and `Session`, and which holds `Foxhound`, `Crawl`, `Label`, `Dataset` and `CrawlContext` (ADR 0095), so a model that reads one entry file finds the other.
- **Foxhound's type `url_pattern` is `link_pattern`** (`LinkPattern` in every language; S3): a glob or a regular expression that a link, a sitemap entry or a kept response's URL (ADR 0110) is matched against in the engine, with no predicate (ADR 0081). TypeScript's `URLPattern` would shadow the web platform's global of that name, whose objects Playwright accepts and Goodfetch does not, which S3 forbids; Collie's `URLMatcher` is another type, since its predicate runs in the program.
- **The AI verbs' doc comments open with their model** (G6, T; finding 1): `act`, `observe`, `extract` and `agent` each start with "Needs a model unless its Plan entry replays: pass `model` or set `ai.model`."

### What stays (finding 4, and two of finding 1)

- **Leaving a Run's scope still stops it with a Checkpoint** (ADR 0078), and only `finish()` ends it finished. Four programs ended a Run by its scope or `close()`, or left `finish()`'s report unused in Rust. A scope that finishes on normal exit and stops on an error, as a transaction commits, was proposed, and is rejected:
  - TypeScript's `Symbol.asyncDispose` is called with no argument, so `await using` cannot tell a normal exit from a throw, and Rust's `Drop` cannot await. Only Python's `__aexit__` sees the exception, so the meaning would differ across languages (S1).
  - Finishing also judges the Run's Verdict and promotes its Plan versions (ADRs 0091, 0102). Doing that because a block ended would be a hidden effect (G2).
  - Instead, `start()`'s and `close()`'s doc comments open with "Leaving the scope or `close()` stops the Run with a Checkpoint; `finish()` ends it", and Rust's `RunReport` keeps `#[must_use]`, the warning that already made two agents read their report (G1).
- **`model_not_configured` stays raised by the call that needs a model** (finding 1). At `start()` the engine cannot know whether each call will pass its own `model`, or whether the Plan replays every step with no model.
- **`model` stays optional on the verbs** (S2, ADR 0083). The type checker cannot see a model set in a project file, configuration or an Operator policy.
- **No model is picked from a provider key in the environment** (ADRs 0043, 0101, G5): a call that costs money names what it calls.

## Considered Options

- **`output` and `partial` kept at the top level, with a discriminant beside them:** rejected. A program that reads `output` still compiles and still loses `partial`, the misread finding 2 measured (G1).
- **The answer on two result types by presence of an output** (`Answered`, `Unanswered`): rejected. It is a second way of saying how the work ended beside `ended` (S2).
- **`gave_up` kept as `finished` with no output:** rejected. `finished` could then hold no answer, and the union would need a third case (G1).
- **A `#[non_exhaustive]` `Ended` in Rust:** rejected. It forces the wildcard arm that tripped `sonnet-gf-rs-11`, for a value that has no third meaning.
- **A Hook in scenario 06 kept as the golden form:** rejected. ADR 0047 makes whole types a setting and finer blocking a Hook (S2); the setting covers every browser fetch with no code, where the Hook was scoped wrong once in three.
- **`run.outcomes({ latest: true })`:** rejected. A sequence that follows new commits cannot take back an outcome a later one supersedes, so the flag would change what the call is (S2).
- **Finishing a Run on normal scope exit in Python and Rust only:** rejected by S1 (above).
- **Raising `model_not_configured` at `start()` whenever a Run names a Plan and no model is set:** rejected. It would refuse Runs whose Plan replays every step with no model, the point of a Plan (ADR 0009).
- **Accepting a host in `domains` and reducing it to its registrable domain:** rejected by G3 and G5.
- **A Scripted model that refuses a data reply to a compile request, naming `model` mode:** rejected. The test would have to switch to `model` mode, not the mode it tests.

## Consequences

- **Amends ADR 0079.** AI work's answer sits in `ended`: `finished` with `output`, `stopped` with its `reason` and `partial`. A note sits under its three fields and its partial results.
- **Amends ADR 0100.** `AgentResult`'s `output` and `partial` sit in `ended`, and a model's `gave_up` stops the task with that reason. Notes sit under "The result" and its stop reasons.
- **Amends ADR 0047.** `block_resources` is the `browser.*` Session setting `browser.block_resources`. A note sits under "Scriptlets and resource types".
- **Amends ADR 0083.** `browser.block_resources` joins the `browser.*` Session settings. A note sits under them.
- **Amends ADR 0111.** Golden scenario 06 blocks images and fonts with `browser.block_resources`; its Rust route is in the `hooks` snippet. A note sits under its "Golden scenario 06".
- **Amends ADR 0102.** The start option is `runId`, and a crawl's Run and `CrawlRunView` gain `failedRequests`. Notes sit under its options and under `retryFailed`.
- **Amends ADR 0105.** The Scripted model takes `scriptedExtraction(data)`, which answers a compiled step too. A note sits under the Scripted model.
- **Amends ADRs 0101 and 0107.** An unpriced `ai.model` under a currency Limit or a price allowance fails the Run's start. Notes sit under "Unpriced means unpriced" and under the allowance's `price`.
- **Adds to ADRs 0056 and 0092.** A host in a source's or a binding's `domains` is refused with `setting_invalid`; nothing they decided changes.
- **Adds to ADR 0078.** The scoped forms' and `close()`'s doc comments state that they stop a Run, and `RunReport` keeps `#[must_use]`.
- **Adds to ADR 0095.** Each package's entry file opens with the map of where the main types live.
- **`GLOSSARY.md`** gains **Run ID**.
- **`TESTING.md`** gains the cases for these shapes.
- **Who builds what.** **Area 11 (Client surfaces)** builds the result types per language, `runId`, `failedRequests`, `scriptedExtraction`, the covariant generics, the doc comments and the entry-file maps, and renames `link_pattern`. **Area 9 (AI and agent)** builds `gave_up` as a stop reason and the testing kit's compile answer. **Area 5 (Site access)** builds `browser.block_resources` in the interception. **Area 3 (Engine runtime)** builds the price refusal at a Run's start and `failedRequests`' read. Each lists this ADR.
- **ADR 0037:** areas 3, 5, 9 and 11 list this ADR, and in its matrix request interception and resource blocking (SA-17), the agent loop (AI-05, V3-04), token and cost accounting (AI-14), resume (RT-10), the failed-request row (RT-04), the typed public API (SDK-06) and docs for AI coding assistants (SDK-07) cite it; no status changes.
- **The pain coverage matrix** closes here: every row reads API, Elsewhere or Declined. P04, P14, P17, P19, P30, P31, P42, P45 and P46 name these shapes; P13, P26, P32, P35, P39, P45, P51 and P56 close as Elsewhere, and P57 as Declined. Golden scenarios 06, 07 and 11 read the new shapes, and their measures change; the scenario points #483's review deferred here (03's Success checks, 09's exclusive `cdp_url`, 10's misses, one price meaning, 07's SIGTERM, two leader scores) are fixed with them, and every Goodfetch failure point of the twelve is typed.
  > Note, 2026-11-11 (the owner, grilling of decision G-19 of the consolidated review, Q31: « Yes, all six », on the option as put): the owner reversed P57's decline for attaching. Electron apps are "taken as attach-only with a conformance case, as for Android Chrome" (ADR 0037's matrix, SA-70; [#485](https://github.com/goodfetch/goodfetch/issues/485)'s note of that day).
- **The `prototype/sdk-api` sketch** at [`41a656f`](https://github.com/goodfetch/goodfetch/commit/41a656f7da0d29821ef7133886f86e55e572abb2) holds these shapes in all three languages and passes `check.sh`; its [pain coverage matrix](https://github.com/goodfetch/goodfetch/blob/41a656f7/sdk-api-sketch/pain-coverage.md) has no blank row.
