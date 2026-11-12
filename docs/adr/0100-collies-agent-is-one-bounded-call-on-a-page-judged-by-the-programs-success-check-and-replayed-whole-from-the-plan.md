# Collie's Agent is one bounded call on a page, judged by the program's Success check and replayed whole from the Plan

A program hands Collie's built-in Agent one objective with `page.agent(task, options)`. The Agent is ADR 0013's thin, journaled loop over the Operation catalog, run as one call that ends in ADR 0079's three separate fields: how it ended, its Verdict and what the model claimed. The program states the Success check with Playwright's `expect` (ADR 0091). A step cap, a stall rule and the task's own timeout stop the task with a reason and its partial answer. The steps are kept as one entry of the Run's Plan and replayed with no model call; on Drift, the loop takes over from the step that drifted. By default the task navigates only on the site it started on. The leaders split on this: Stagehand removed `agent()` in v4 and Playwright removed its hidden `page.agent()` after one release, while Browser Use, Skyvern and Notte keep a loop. False successes and loops that never say why they stopped are among the research's most-reported pains, and so are replay without the model and caps on what a loop spends ([AI browser automation](https://github.com/goodfetch/goodfetch/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/ai-browser-automation.md) §4.1 item 6, §4.2 and §6 pains 2 and 8, then 1 and 13). The loops' exact defaults were checked at tagged releases in [Research: agent loop APIs](https://github.com/goodfetch/goodfetch/blob/research/sdk-agent-loops/rnd/research/2026-10-15-sdk-agent-loops.md). Decided in [The Agent in Collie's SDK API](https://github.com/goodfetch/goodfetch/issues/474), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/goodfetch/issues/461), every answer decided by the agent on the owner's delegation, with its [grilling record](https://github.com/goodfetch/goodfetch/issues/474#issuecomment-6181861259), [round 2](https://github.com/goodfetch/goodfetch/issues/474#issuecomment-6187826787) and [round 3](https://github.com/goodfetch/goodfetch/issues/474#issuecomment-6188859562). Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md). The page is [ADR 0085](0085-collies-page-api-keeps-playwrights-names-strictness-waits-values-cross-as-json-and-unanswered-requests-fall-through.md)'s, the AI verbs, Plans and `expect` are [ADR 0091](0091-collies-ai-verbs-live-on-the-page-replay-from-a-plan-file-the-run-names-and-expect-runs-a-success-check.md)'s, Sessions and secrets are [ADR 0092](0092-sessions-open-from-their-source-page-pause-waits-for-a-person-and-a-secret-is-a-placeholder-bound-to-domains.md)'s, models and Usage are [ADR 0101](https://github.com/goodfetch/goodfetch/pull/515)'s, a crawl handler's page is [ADR 0093](https://github.com/goodfetch/goodfetch/pull/512)'s and Politeness is [ADR 0086](0086-a-crawl-obeys-robots-txt-and-sends-one-request-at-a-time-per-domain-by-default-and-pressing-harder-is-recorded.md)'s.

> Note (owner's decisions, 2026-11-11, grilling of decision G-17 of the consolidated review: Q4 answered "Yes, all eight" and Q11, put after an independent check of the grilling's drafts in correction of Q4's Evaluation, answered "Yes, all three", each on the grilling's recommended option as put; the wording is the grilling session's, and the owner did not read it): the step loop stands and is what line 4 of Features builds ([#957](https://github.com/goodfetch/goodfetch/issues/957)), before any second way. Once Code mode exists ([#1039](https://github.com/goodfetch/goodfetch/issues/1039)), a second way to run a task is added behind one setting, in which the model writes a short program each turn, under the same bounds and the same Effect journal; no ticket builds it yet. The default stays the step loop until an Evaluation on the same tasks shows the program way verifies more tasks, with no false success; that result goes to the owner, and nothing switches by itself (Q4). The Evaluation uses the Proving ground's Agent tasks, graded by the oracle and never by a model, three trials each, on the Vercel AI Gateway's free-tier models only, one small and one larger. A run cut short by the free tier's limits counts neither for nor against; a result reaches the owner only once both ways finished every trial on at least the small and the larger model, and it holds for those models only (Q4, Q11). The fact this ADR had not weighed, as Q4 put it: browser-use 1.0.0, on 2026-11-09, replaced its step loop with a program per turn.
>
> Note (the grilling session's own, 2026-11-11, same grilling, awaiting the owner's reading; the setting's name was shown to the owner as point S5 of the table the owner confirmed, "Yes, confirmed"): the setting's proposed name is `agent.loop`, with the values `steps`, the default, and `program`, a shape for the ticket that builds the second way. The browser-use fact rests on `MIGRATING.md:3-5` of browser-use 1.0.0, read in the review's clone: "The agent no longer picks one action per step from a DOM dump. It writes Python in a persistent REPL". A free-tier limit is a `429` or a spent free credit, which ADR 0076 already makes a run inconclusive.

## Decision

### One call on a page

- **The SDK offers the built-in Agent** (G1, G2, S2). ADR 0013 rejected "no built-in agent" and gives it "the user's objective tasks", and ADR 0037 decided AI-05 and V3-04. The research found no GitHub complaint about Stagehand's removal, only one HN question, while the embedded loops are where the false-success and hang reports are (AI §4.2). The Verdict and the bounds below answer those. Each call runs one task to its end, and nothing carries into the next call: a remembered conversation that a follow-up task silently continues, as Browser Use's `add_new_task`, would be hidden state (G2).
- **`page.agent(task, options)`, on the page only** (G2, S1, S3, T). The task starts on a page the program holds, so a Borrowed page (`run.borrow(page)`, ADR 0078), a crawl handler's page and a popup get it, as they get ADR 0091's verbs. Skyvern's `page.agent.run_task(prompt)` also runs on the page the program holds and hands control back "on the same page".
  - **Not an agent object configured, then run**, as Browser Use's `Agent(…).run()`, Notte's `Agent(session).run(task=…)` and Stagehand v3's `agent({…}).execute(…)`. That would be one more value and lifecycle (S2, T), and `run` already names a Run in Goodfetch (S3). A model that writes Stagehand's form meets a type error first (G3).
  - **Not on a locator,** whose scope would end at the first navigation, and **not on the Run,** which would deny the program logging in or navigating before it hands over.
- **The page and its tabs.**
  - The task acts on its page and on the tabs it opens, never on the program's other pages.
  - The whole task is one gesture under ADR 0085's per-page order: the program's later gestures on that page wait for it, while reads and waits run. While the task waits on the program, running an Agent tool or the Success check, it gives up the page's turn between its own inputs, as ADR 0085's locator handlers and ADR 0092's `login` do, so the gestures queued meanwhile run, the tool's own among them.
  - When the task ends, the tabs it opened are closed as `page.close()` closes them, so on a Borrowed browser they are only released (ADR 0085). The page it ended on stays open, and the result returns it (G4).
  - On a crawl handler's page the task is an acting AI verb, so under `fetch.tier: auto` it escalates to the browser, and under a forced tier it fails with `browser_required` (ADR 0093).
- **What the call takes** (G3, S2). `page.agent(task, { schema, successCheck, maxSteps, domains, tools, variables, model, timeout, signal })`:
  - `task` is the objective in words. Its `%name%` placeholders follow ADR 0091: `variables` are plain values, as `act` takes them, and a placeholder that no variable fills is a Secret placeholder bound by `secrets.<name>` (ADR 0092). The model, the Plan and the logs see only `%name%`.
  - `schema` is the output's schema in ADR 0082's forms. Without one, the output is nothing, and the model's message is in `reported`.
  - `maxSteps`, `domains`, `model` and `timeout` are per-call arguments, each that setting at the run layer (ADR 0083): `agent.max_steps`, `agent.domains`, `ai.model` and `timeout.agent`. There is no Agent-only model key, since `model` already picks a model for one task (ADR 0101, S2).
  - `agent.max_stalled_steps` and `agent.human_pause` are settings only. The system prompt, vision and the Decision provider stay settings of their scopes (ADRs 0013, 0083, 0101).
  - Python spells the options `success_check`, `max_steps` and so on, as keyword-only arguments. Rust spells them as builder methods.

### The Success check

- **The program states it with `expect`** (G1, G6, S3, T): `successCheck: async (page) => { await expect(page.locator(".breadcrumb")).toContainText("Travel"); … }`.
  - Each `expect` in the function is a Success check the engine evaluates live and records (ADR 0091). Its timeout is `timeout.wait` as anywhere, and a program that expects the condition at once can give it a shorter one.
  - The function runs where the program's code runs (ADR 0081), so it can state any condition Playwright's assertions state, over several elements.
  - A declarative value (`[check.url(…)]`) would name Playwright's conditions anew, which ADR 0091 rejected for single Success checks (S3). A model-judged check, as Browser Use's judge and Notte's `CompletionValidator`, is ruled out by ADR 0013.
  - The function is not data, so the CLI and MCP state their Success checks through the catalog's `expect` operation instead. A Plan never holds it: ADR 0009 says "a Success check is never cached".
- **When it runs.** Each time the model reports done, on the page the task is on.
  - When it passes, the task finishes.
  - When an `expect` in it fails, the failure goes back to the model as the answer to its "done", and the loop goes on within its bounds. The failure is the Success check as rendered, with what was observed. Notte's `CompletionValidator`, an LLM, can reject a "done" the same way, and Pydantic AI's output validators send a program's objection back with `ModelRetry`; here it is the program's own condition, evaluated by the engine.
  - It does not run when the model gives up or the task stops early.
- **The Verdict reads its latest evaluation** (ADR 0029). The task is `verified` when that evaluation passed and every other check the task relies on holds. It is `rejected` when that evaluation failed. It is `needs-review` when the Success check never ran or the task declared none (ADR 0013).
- **What the function raises** (G1):
  - A `PageError`, `success_check_failed` or another one such as a target that is not unique, is the condition's answer, above: the page is not in the state the program stated.
  - Another `GoodfetchError` ends the task with its own code, as ADR 0079 keeps a handler's.
  - A cancellation is never wrapped: someone else stopping the task makes it `stopped` (below), and the caller's own cancellation keeps its language's form (ADR 0080).
  - Any other exception, or in Rust an error that is not a Goodfetch one, is the program's bug, never a rejection: the call raises `success_check_threw`, a `ProgramError` chaining it and naming its type and message.

### The result

- **`AgentResult<T>` is ADR 0079's `AiWorkResult<T>`** (`ended`, `verdict`, `reported`, `output`, `partial`, `requireVerified()`), plus three fields (G1, G6, S1):
  - `steps`, every step of the task;
  - `usage`, ADR 0101's `ModelUsage` for the whole task;
  - `page`, the page where the task ended.
  > Amended by [ADR 0117](0117-what-coding-agents-misread-is-reshaped-an-ai-result-holds-its-answer-in-how-it-ended-resource-types-are-blocked-by-a-setting-and-a-start-names-its-run-id.md): `output` and `partial` sit in `ended`: `finished` with `output`, `stopped` with its `reason` and `partial` (narrowed on `ended.kind` in TypeScript, the classes `Finished` and `Stopped` in Python, an exhaustive `Ended<T>` in Rust).
- **`output`** is a complete `T`: the model's final answer, checked against the schema and the program's validators (ADR 0082) within ADR 0012's bounded retries. An answer that never passes raises `model_output_invalid` (ADR 0101).
- **Grounding holds the output's values** (ADR 0013).
  - Each value must appear in a page the task observed, or in the result of one of its Agent tools (the program's own data), or follow from a declared transform. Grounding records where each value was found, which is how a Plan reads it again.
    > Amended by [ADR 0123](0123-a-typed-value-read-from-page-text-comes-from-an-engine-transform-under-a-stated-locale-and-time-zone.md): a declared transform is one of ADR 0123's Value transforms other than `map`, whose table no Plan review would check here, and the Agent answers each field with the text it read and its transform, from which the engine computes the value.
  - A value that does not is fed back once through ADR 0012's retry. If it stays ungrounded, the output is returned with `verdict: needs-review` naming the field, since a valid `T` the checks cannot prove is a Verdict's business (ADR 0029).
- **`reported`** is the model's claim, `done` or `gave_up` with its message. It is absent when no model claimed anything in this task: it stopped before a claim, or it was replayed whole with no model call.
- **`ended`** is `finished` when the model reported done or gave up, or when a replay reached its end and its Success check ran (a task that declares none is never `verified`, so its entry is never promoted and never replayed); otherwise `stopped` with its reason (below).
  > Amended by [ADR 0117](0117-what-coding-agents-misread-is-reshaped-an-ai-result-holds-its-answer-in-how-it-ended-resource-types-are-blocked-by-a-setting-and-a-start-names-its-run-id.md): a model that gives up stops the task with the reason `gave_up`, carrying `partial`, so `finished` always holds a complete output; `reported` still holds the claim `gave_up` and its message.
- **`partial`, from a `note` action** (G1, S1). The Agent has a built-in action that notes what it has found so far against the schema. `partial` holds the latest noted or answered value of each top-level field, or of each element for a list output, that is valid against its own part of the schema and Grounded, and nothing else. It is set only when `output` is not.
  - TypeScript types it `PartialOf<T>`: `Partial<T>` for an object, the elements for a list.
  - Python gives a mapping or a list of validated values.
  - Rust gives `PartialOutput`, with typed accessors.
  > Amended by [ADR 0117](0117-what-coding-agents-misread-is-reshaped-an-ai-result-holds-its-answer-in-how-it-ended-resource-types-are-blocked-by-a-setting-and-a-start-names-its-run-id.md): `partial` sits in `ended`, when the task `stopped`.

  Stagehand v3 returned `completed: false` with the actions taken, and Notte `success=False` with its answer. The AI SDK's `.output` throws when the last step ended on tool calls.

### Bounds

- **A step cap, not a Limit** (G5, S2, S3). `agent.max_steps` is **20** by default, the value the AI SDK's `ToolLoopAgent`, Stagehand v3, Midscene and Notte converged on; `maxSteps` sets it for one task. The others range from Playwright 1.58's 10 actions and OpenAI Agents' 10 turns to Browser Use's 500.
  - A step is one model decision and the actions it takes, or one step replayed from the Plan. A Human pause is no step (ADR 0092).
  - Reaching the cap stops only the task, and the Run goes on, whereas a Limit stops the Run (ADR 0080). That is why ADR 0080's rejected step Limit is not this cap.
- **A stalled loop stops** (G1, G6). `agent.max_stalled_steps` is **3** by default, as Notte's `max_consecutive_failures`. A step stalls when its actions failed, or when it repeated earlier actions on targets whose page had not changed since; a "done" that the Success check rejects again on an unchanged page is such a repeat. Browser Use's "Step 1" loops, its most-commented issues (#839, #1020), ran on without saying why; the stall rule ends a loop that makes no progress, and ADR 0101's typed provider failures end one whose model fails.
- **The task's own timeout** (G6). `timeout.agent` is **10 minutes** by default, and `timeout` sets it for one task. It nests the timeouts of the task's actions and model calls (ADR 0101's `timeout.model`); ADR 0091's `timeout.ai` bounds a single AI verb and does not apply to the task.
  - When it passes, the step in flight is cancelled, as nesting requires, and an Effect already sent is never treated as undone (ADR 0080). The task returns `stopped` with the reason `timeout`.
  - An enclosing timeout that ends the task first, such as a crawl handler's `timeout.handler` (60 s), fails it with `deadline_exceeded` naming the enclosing call (ADR 0080), since that call fails too. A Label that runs an Agent sets a longer `timeout.handler`.
  - Time in a Human pause does not count (ADR 0080).
- **Every early end of the task is a value** (G1, S2). As ADR 0079 makes AI work stopped by a Limit, the result is `ended: { stopped, reason }`, with `steps` and `partial` kept. A program reads every early end in one place. The reasons:
  - `max_steps`: the step cap, naming the setting and its layer;
  - `no_progress`: the stall rule, naming the setting and its layer;
  - `timeout`: the task's own `timeout.agent`, or the Run's timeout, naming the setting;
  - `limit`: a Limit by name, after which the Run stops too (ADR 0080);
    > Amended by [ADR 0107](0107-a-runs-model-allowance-refuses-its-model-calls-once-spent-while-the-run-goes-on.md): `model_allowance`: the Run's Model allowance (`ai.allowance.*`) is reached, naming the setting, its value and its layer; the Run goes on, and a task that starts after it is reached returns it with no step.
  - `abort`: someone other than the caller stopped the task, with who: a person in the Console, another Client, or the program closing the page or the Run.
    > Amended by [ADR 0113](0113-a-human-pause-whose-page-closes-raises-call-cancelled-naming-what-closed-and-a-lost-live-view-ends-no-pause.md): `by` also takes `browser`, for a page or browser closed in the browser outside every Client.
    > Amended by [ADR 0137](0137-the-console-leaves-both-products-and-the-engine-and-the-owners-monitoring-tool-is-designed-apart.md): no Product carries a Console. The engine behaviour this ADR describes stands and the Console's part does not; a Client reaches it.
  > Amended by [ADR 0117](0117-what-coding-agents-misread-is-reshaped-an-ai-result-holds-its-answer-in-how-it-ended-resource-types-are-blocked-by-a-setting-and-a-start-names-its-run-id.md): `steps` stay on the result and `partial` sits in `ended`, and a model that gives up is one more early end: `gave_up`, the model reported that it gave up, its message in `reported`.

  The leaders split here: OpenAI Agents, Pydantic AI, Midscene and Playwright 1.58 raise at their cap, while the AI SDK, Stagehand v3, Notte and Browser Use return. The caller's own cancellation keeps its language's form (ADR 0080).

### Steps as they happen

- **A lossless sequence, from the task's first step** (G1, G6). ADR 0081 makes an Agent's steps "a sequence read to its end". Every step is a record a program may log or show, so none is dropped, and an unread open iterator slows the task (ADR 0081). Each `steps()` call is a new reader that starts at step 1 whenever it is called, so nothing slips past a late reader, as ADR 0080's waiters arm when called. The iterator ends when the task ends, and raises the task's error when it failed.
- **One call returns a task handle,** shaped per language (S1 over S4, T). The leaders that stream a model loop give one value to iterate and then ask for the final result: OpenAI Agents' `StreamedRunResult` and `run_streamed()`, and the AI SDK's `stream()`; Anthropic's `messages.stream()` with `finalMessage()` has the same shape for one streamed message.
  - **TypeScript:** `page.agent(…)` starts the task and returns an `AgentTask<T>`, a `Promise<AgentResult<T>>` that also has `steps()`, as openai-node's `APIPromise` adds methods to a promise. `await page.agent(…)` is the whole program in the common case. A failure that reached the program through `steps()` counts as handled, so a program that reads only the steps meets no unhandled rejection.
  - **Python:** `page.agent(…)` returns an `AgentTask`, or an `AsyncAgentTask` in the async API, with `result()`, `steps()` and a `with` block (`async with`), as Anthropic's and OpenAI's stream helpers do. It starts at the first of `result()`, `steps()` or entering the block. Leaving the block before the task ends cancels it, and a handle released without being started logs a warning, as a coroutine never awaited does. The sync and async APIs keep one shape: `await page.agent(…).result()` and `page.agent(…).result()`.
  - **Rust:** `page.agent(task)` is a builder that `.await`s to the result, as ADR 0080's calls do. `.start().await?` gives an `AgentTask` with `steps()`, an `EventStream`, and `result()`.
- **An `AgentStep`** carries:
  - its number;
  - its `origin`, `model` or `plan`, and on the first live step after the replay ended, why it ended, as ADR 0091's `drift`: the target the Plan expected and what the page showed, or the Success check that failed and what it observed;
  - the model's own message, a signal, empty for a replayed step;
  - its actions, each with the catalog operation, its target as a `Locator`, the call rendered in the caller's language as `code` (ADR 0091's renderer), and its outcome: done, failed with its error, or already in the Effect journal;
  - the page's URL after the step;
  - whether it stalled;
  - its `ModelUsage`.

### Agent tools

- **The program adds its own actions** (G1, G3, S2). An **Agent tool** is a function of the program that the model may call as an action during the task, such as a lookup in the program's database. Each language spells it its own way: `agentTool({ name, description, input, readOnly, category, run })` in TypeScript, the `@agent_tool` decorator in Python, and `AgentTool::new` in Rust.
  - Its input schema takes ADR 0082's forms and is checked before it runs.
  - It runs where the program's code runs (ADR 0081), with the input and the task's current page, while the task gives up the page's turn.
  - It returns JSON the model reads, bounded by `observation.max_size` (ADR 0091).

  OpenAI Agents' `@function_tool`, Pydantic AI's `@agent.tool`, the AI SDK's `tool()` and Browser Use's `@tools.action` are the forms models know. Browser Use injects parameters by name, and a wrong name fails silently.
- **An Effect unless `readOnly`** (G5). As ADR 0011 classes a site's WebMCP tools, an Agent tool is journaled and never redone (ADR 0010). A `category` from ADR 0014's list asks a person for confirmation before each call unless the run allows that category (ADR 0014). Payment and accepting terms always ask, since their allowances name domains (ADRs 0045, 0048) and a tool has none.
  > Amended by [ADR 0106](0106-a-program-declares-an-effect-as-a-keyed-block-whose-output-the-effect-journal-keeps-judged-by-what-the-engine-sent.md): a tool that is not read-only may be `repeatable` (`repeatable: true`, `repeatable=True`, `.repeatable()`), and a raise from it then leaves no uncertain Effect, so a replay or a later task may call it again. Its call carries an `idempotencyKey` derived from the tool call's Effect journal entry: `call.idempotencyKey` in TypeScript, an `AgentToolCall` third parameter in Python and Rust.
- **What a tool raises** (G1), by the Success check's rule:
  - a `GoodfetchError` ends the task with its own code;
  - a cancellation is never wrapped;
  - any other exception, or in Rust an error that is not a Goodfetch one, ends the task with `agent_tool_failed`, a `ProgramError` chaining it and naming its type and message.
    > Amended by [ADR 0109](0109-a-rust-programs-own-code-returns-any-error-as-a-box-error-read-as-the-other-sdks-read-an-exception.md): in Rust an Agent tool and the Success check return a `BoxError`, read by the outermost error; the type they name from Rust reads `BoxError`, and the message carries the error's `source()` chain.

  A bug must not read as the model's problem. An answer the model should act on is a value the tool returns; in Pydantic AI too, an exception other than `ModelRetry` or `ToolFailed` ends the run. A tool that is not read-only and raised leaves its Effect uncertain in the Effect journal (ADR 0010), so the Run's Verdict waits for a person to reconcile it (ADR 0029); `agent_tool_failed` says so in its details, and whatever error ends the task lists the tool in `agentTask`.
  > Amended by [ADR 0106](0106-a-program-declares-an-effect-as-a-keyed-block-whose-output-the-effect-journal-keeps-judged-by-what-the-engine-sent.md): a raise from a `repeatable` tool leaves no uncertain Effect.
- **Two tools with one name, or a tool named like a built-in action,** are refused with `agent_tool_invalid`, a `ProgramError`, before the task starts (G3).
- **No MCP servers as tools.** A tool's function can call any MCP client the program uses, so the SDK needs no new type for it (G7, S2).

### The Plan

- **One Plan entry per task** (G2, S2). ADR 0091 keys Plan entries per call and leaves "a Plan replayed toward a goal", which "keeps ADR 0009's whole-sequence rule", to this ticket; golden scenario 11 asks to "keep the steps taken so that they can be repeated without a model".
  - The entry lives in the Run's Plan file (ADR 0091's `plan`).
  - It is keyed by the verb, the task with its placeholders, the hash of the schema's JSON Schema and the names of the program's tools, with one variant per start page layout.
    > Amended by [ADR 0123](0123-a-typed-value-read-from-page-text-comes-from-an-engine-transform-under-a-stated-locale-and-time-zone.md): when an output field holds a locale-dependent Value transform, the start page variant also holds the start page's document language, the Session's `browser.locale` and the `extract.locale` in effect, so a start page in another language is a new variant rather than a replacement of the entry's steps, and for a date-time or relative time the Session's `browser.timezone_id` and `extract.timezone_id`; each output field keeps the language of the element it was read from, and a replay that reads it from an element in another language while the document's did not change is Drift, so the live loop takes over.
  - It holds the actions that succeeded, in order, each as ADR 0091's `act` step: its frame path, locator ladder, Target fingerprint, method and arguments with placeholders, and Effect class, and the last Effect before the recorded run first observed its target.
  - It holds the tool calls with their arguments.
  - Per output field, it holds the step at which Grounding found the value and the locator ladder and transform it was read with, so the output is read again at that step with no model, as ADR 0013's compiled mode. A value found in a tool's result is taken from that tool call again.
- **Replayed whole, under ADR 0009's rule read across pages** (G1).
  - ADR 0009 checks every step's target before the first Effect. A task that crosses pages has steps whose page exists only after an Effect, such as a confirmation page after an order is placed, and no replay can observe that page beforehand.
  - So, before each Effect, the replay checks against their Target fingerprints the Effect's own target and every remaining step's target that the recorded run observed before this Effect on a page already open, having checked every earlier step as it replayed it.
  - Any other step is checked when the replay reaches it, before it acts: one on a page not yet open, or one whose target the recorded run first saw after an Effect, such as a "Checkout" button that appears once an item is in the cart. The replay never navigates ahead to look, and a target the site shows only after an Effect is not taken for Drift before it. A Drift seen before an Effect therefore stops the replay before anything changes on the site, which is the lab failure ADR 0009 guards against, a consent box ticked before the Drift was seen.
  - Then the output is read and the Success check runs, as in a live task. Replayed steps report `origin: plan`, with no model call.

  A replayed task repeats the decisions the model made, and the Success check is what vouches for them. Browser Use's `rerun_history` still calls a model for extraction and a summary, and its beta ignores the recorded run; Skyvern and Notte turn a run into code on their servers. "Replay without the model" is AI pain 1.
- **On Drift, the loop takes over, with no Repair.** The replay ends at the step that drifted, and the task goes on live with the model from there, within its bounds. The steps already replayed stand, and the live steps replace the rest of the entry.
  - ADR 0009's Repair recompiles a Plan whose steps it can all observe and checks the result in full before it acts. An objective task can only be recompiled by observing the pages it reaches, which is what the live loop does.
  - What keeps it safe: every live step acts on the page as observed now; the Effect journal keeps an Effect from being sent twice (ADR 0010); the new steps become a Plan version only after a `verified` Run, and a Plan with Effects only after review (ADRs 0009, 0091); and the Success check vouches for the result.
- **A replay whose Success check fails** hands over the same way: the loop takes over from the replay's end, with the failure as its first observation.
- **Without a model.** Under `plan.frozen`, or with no model configured:
  - a Drift raises `plan_drift` (ADR 0091);
  - a missing entry raises `plan_step_missing` under `plan.frozen`, and `model_not_configured` with no model configured (ADR 0091);
  - a failed Success check after the replay finishes the task, `finished` with the Verdict `rejected`.
- **Promotion follows ADR 0091:** a new version is promoted after the Run finishes `verified`, with ADR 0009's review of Plans with Effects. Without `plan`, the sequence lives only in the Run trace.
- **A task graduates to code.** Every page action in `steps` carries its `code` in the caller's language, so a person can turn a task into plain page API calls. That is the research's most-liked quality: generate with AI, then own deterministic code (AI §5, like 1).

### Where the Agent may go

- **`agent.domains`, by default the registrable domain of the page the task starts on** (G5). `domains` sets it for one task. It takes no `null`: `["*"]` is the explicit way to lift it.
  - An entry matches its host and their subdomains. `*` allows wherever the run's domain policy allows, and applies only when written.
  - It bounds where the task navigates, in its page and the tabs it opens, never the subresources a page loads, which the run's domain policy (ADR 0014) governs. The engine enforces it as it enforces that policy.
  - A navigation outside it is refused, and the refusal goes back to the model as a failed action.

  Browser Use's `allowed_domains` and Stagehand's and Skyvern's domain filters were bypassed (AI-9; Stagehand #2945, Browser Use #5099, Skyvern #8577), which is why ADR 0014 enforces policy in the engine. Golden scenario 11 says "Stay on books.toscrape.com, whatever the pages suggest".
- **No robots.txt opt-in.** ADR 0086 left "any opt-in to obey robots.txt" for an Agent's own navigation here. Declined:
  - An Agent's navigation is the program's own (ADR 0086), working for one person toward one task. Google's user-triggered fetchers generally ignore robots.txt, and OpenAI says it may not apply to user-initiated actions.
  - robots.txt is still read and recorded in Provenance, and `agent.domains` already bounds where the Agent goes.
  - On a crawl handler's page, the task's navigation is the handler's, governed by Politeness and robots.txt (ADRs 0086, 0093). Its `site_preferences_refused` is a failed action given back to the model, as every refusal inside the task is.

  A later ticket reopens it with new evidence.
- **`agent.human_pause`, `false` by default** (G5). When on, the model may ask a person through a Human pause (ADR 0092), with its reason shown in "Needs you" (ADR 0031). The person's answer, a decline with its note, or the pause running out (`timeout.human_pause`) goes back to the model as that action's outcome: the model asked, and a refusal is an answer. ADR 0092 ends a call when a pause the engine raised inside it runs out; this pause is a question the model asked, so it amends ADR 0092 for it. It is off by default because an unattended program would wait up to 24 hours.
- **Confirmations.** Effects in ADR 0014's categories still ask for confirmation unless the run allows them. A person declining ends the task with `effect_not_confirmed`, as for `act` (ADR 0091), so the model never tries another way to do what a person refused.
- **`session.recovery.agent`, `true` by default.** It switches ADR 0006's AI agent login step, as ADR 0092's `session.recovery.human_pause` switches the Human pause step.
  - The step runs the built-in Agent with the Session's Secret placeholders, bound by their own domains (ADR 0092) and the run's domain policy, not by `agent.domains`, so a login that passes through another domain works.
  - It is skipped when no model is configured and no login Plan replays it, or under `plan.frozen` without one, and `logged_out` (ADR 0092) lists why.
- **What the model reads.** Page text is marked untrusted (ADR 0014). Observations are ADR 0091's `ai` aria snapshots, bounded by `observation.max_size`, so a page too large for the model collapses its deepest subtrees instead of failing.

### Failures

Under ADR 0079's rule, the call raises when it cannot keep its promise, and AI work that ends early is a value:

| Case | Result |
|---|---|
| A failed page action, a refused navigation, an answer that fails Grounding, a failed Success check, the person's answer to the model's own Human pause | not raised: the step's outcome, given back to the model |
| The step cap, the stall rule, the task's timeout, a Limit reached during the task, someone else stopping it | a value: `ended: stopped` with its reason |
| An enclosing call's timeout | `deadline_exceeded` naming that call (ADR 0080) |
| A Limit the task draws on, already used up when it starts | `limit_reached`, a `RefusedError` (ADR 0080) |
| A task the Plan holds no entry for, with no model configured | `model_not_configured` (ADR 0091) |
| The Model provider fails after ADR 0012's retries; an answer that never passes the schema | ADR 0101's codes, such as `model_unavailable` and `model_output_invalid` |
| `plan.frozen` with no entry; a Drift under `plan.frozen` or with no model | `plan_step_missing`; `plan_drift` (ADR 0091) |
| An Effect sent and left uncertain | `effect_uncertain`, a `PageError` (ADR 0091): the task ends, so the model never tries the Effect again (ADR 0010) |
| A person declined a confirmation | `effect_not_confirmed`, a `RefusedError` (ADR 0091) |
| A `GoodfetchError` from an Agent tool, or one from the Success check function that is not a `PageError` | that error, with its own code |
| Another exception from an Agent tool; two tools share a name or take a built-in's | `agent_tool_failed`; `agent_tool_invalid`, each a `ProgramError` |
| Another exception from the Success check function | `success_check_threw`, a `ProgramError` |
| A placeholder neither filled nor bound, a variable no placeholder uses, or a name that is both | `variables_mismatch` (ADR 0091) |

> Amended by [ADR 0114](0114-a-program-and-collie-take-turns-on-a-page-other-code-drives-and-an-ai-verb-fails-when-a-navigation-it-did-not-start-replaces-its-document.md): on a page other code also drives, a navigation to a new document that the task did not start is not raised: it is a step's outcome given back to the model, naming both addresses, and a task replayed from the Plan stops replaying there for the live loop, as on Drift, or under `plan.frozen` or with no model configured fails with `page_navigated_during_call`.

**What a raised error carries** (G1, G6). An error raised after steps were taken carries the task's `usage` (ADR 0101). It also carries `agentTask`: the steps taken, the partial answer as JSON, valid and Grounded as `partial` is, and the Agent tools whose Effect the task left uncertain. ADR 0079 asks that data gathered before a failure travel on the error. The steps are also in the Run trace under the error's `callId`.

### Outside agents

- **Unchanged from ADR 0011** (S2). The Agent is one more catalog operation, classed Effect, so the CLI, MCP and the Framework adapters reach it where their profiles include it. How those surfaces state a Success check inside the task is their own design ([Mapping the catalog to each language and surface](https://github.com/goodfetch/goodfetch/issues/471)); until then, they evaluate the catalog's `expect` after the task returns.
  > Amended by [ADR 0099](0099-every-surface-spells-each-catalog-name-by-its-own-rule-shapes-come-from-declared-kinds-and-call-is-the-way-down.md): the Agent's MCP tool is `page_agent`, and a CLI command for it, where area 11's spec offers one, reads `page agent`; which MCP profiles include it, and how the CLI and MCP state a Success check inside the task, are area 11's spec, and until then the interim above holds.
- **An outside agent** drives the same page operations the built-in Agent uses, reads ADR 0091's `ai` aria snapshots with element references, and states its Success check through the catalog's `expect`. Its run gets the same Verdict, and no model's word makes it `verified` (ADR 0029).
- **A program running its own loop** (OpenAI Agents, the AI SDK) uses `ariaSnapshot({ mode: "ai" })`, `getByRef`, the page API and `expect` from the SDK, or ADR 0011's Framework adapters.

## Considered Options

- **No Agent in the SDK,** as Stagehand v4 and Playwright: rejected. It would reverse ADRs 0013 and 0037, which the map's Notes leave to the owner, and golden scenario 11 would need every program to write its own loop.
- **An agent object configured once, then run,** as Browser Use, Notte and Stagehand v3: rejected by S2 and S3 (`run` names a Run). **Follow-up tasks** over a remembered conversation: rejected by G2.
- **A declarative Success check value:** rejected by S3, as ADR 0091 rejected one for single Success checks. **A model-judged check:** rejected by ADR 0013.
- **Raising at the step cap,** as OpenAI Agents, Pydantic AI, Midscene and Playwright 1.58: rejected. AI work stopped early is a value (ADR 0079), and a raised cap would lose the partial answer scenario 11 asks for, or move it into an error's details. **Raising `deadline_exceeded` at the task's own timeout, or `call_cancelled` when someone else stops it** (ADR 0080's rule for other calls): rejected for the same reason. One way to read "ended early" (S2).
- **A step Limit** in ADR 0041's ledger: rejected, as ADR 0080 rejected it. The cap bounds one task and stops only it, while a Limit stops the Run.
- **Browser Use's 500 steps,** or none: rejected by G5. **OpenAI Agents' 10 turns or Playwright's 10 actions:** the low end of the leaders' range; four of them converged on 20 (S3).
- **`partial` as the model's last answer in raw JSON:** rejected by G1, since values nothing vouches for would read as found. **No `partial`:** rejected, since scenario 11 asks for one.
- **Steps only on the result,** or a step listener: rejected by ADR 0081, which makes them an iterator. **Two methods**, one for the result and one streaming, as OpenAI Agents and the Anthropic SDK: rejected by S2, since one handle serves both.
- **A tool's exception fed back to the model,** as OpenAI Agents and Browser Use: rejected by G1. A bug would hide in what the model reads, and OpenAI Agents now sends only a fixed message. **Wrapping a tool's `GoodfetchError`,** as `hook_failed` does: rejected, since a person stopping the task mid-tool would then read as the program's bug. **MCP servers as tools,** as Browser Use and Stagehand v3: rejected by S2 and G7.
- **ADR 0009's Repair for an Agent's task,** a recompile checked in full before it acts: rejected. The pages a task reaches cannot be observed before it acts; the live loop with the Effect journal, the Verdict and review is the bound. **Replay by position with a model fallback,** as Browser Use's `rerun_history`: rejected. It still calls a model for extraction, so it is not replay without the model.
- **No domain bound beyond the run's policy:** rejected by G5, because a model reading untrusted pages decides where to go. **Bounding subresources too:** rejected; a page's own CDN and third-party requests would fail. **A robots.txt opt-in:** declined above.

## Consequences

- **Amends ADR 0079.**
  - AI work's stop reasons gain `max_steps` and `no_progress`, `timeout` covers an Agent's own timeout as well as the Run's, and `abort` says who stopped it.
  - `reported` is absent when no model claimed anything: the work stopped before a claim, or was replayed whole.
  - `partial` holds the top-level fields, or the elements of a list, that are valid against their part of the schema and Grounded.
  - An error raised by `page.agent` after steps were taken carries `agentTask`: the steps, the partial answer and the Agent tools whose Effect it left uncertain.
  - `agent_tool_failed` and `success_check_threw` always name the program's exception type and message, as `handler_failed` and `hook_failed` do.
- **Amends ADR 0080.**
  - `timeout.agent` is a phase, 10 minutes.
  - An Agent's task that reaches its own `timeout.agent` stops with `ended: stopped`, reason `timeout`, instead of raising `deadline_exceeded`.
  - An Agent's task stopped by someone other than the caller returns `stopped` with reason `abort` instead of raising `call_cancelled`.
  - The Agent's own step cap is `agent.max_steps`, which is not a Limit.
- **Amends ADR 0083.** `timeout.agent` joins the timeout family. `agent.max_steps`, `agent.max_stalled_steps`, `agent.domains`, `agent.human_pause` and `session.recovery.agent` join the settings schema. The call's `maxSteps`, `domains`, `model` and `timeout` are those settings at the run layer.
- **Amends ADR 0009.**
  - For an Agent's task, whose replay may cross pages, its Target fingerprint check before the first Effect reads: before each Effect, the Effect's own target and every remaining target the recorded run observed before it, on a page already open, are checked; any other step is checked when it is reached, before it acts.
  - An Agent's task gets no Repair: on Drift, or on a failed Success check after a replay, the loop takes over from there, and its steps replace the rest of the entry.
- **Amends ADR 0006.** `session.recovery.agent` switches the AI agent login step of the recovery ladder.
- **Amends ADR 0013.** An Agent's output may also be Grounded in the result of one of its Agent tools, the program's own data, and `GLOSSARY.md`'s **Grounding** says so.
- **Amends ADR 0086.** An Agent's own navigation offers no robots.txt opt-in.
- **Amends, once they are on `main`, ADR 0091, ADR 0092 and ADR 0093**, each with a note added when this ADR merges after them:
  - ADR 0091: `variables` are taken by `page.agent` too, and its whole-sequence rule reads as above;
  - ADR 0092: `session.recovery.agent` switches the AI agent login step, and the Agent's own Human pause (`agent.human_pause`), a question the model asked, gives its answer, a decline or its running out back to the model instead of ending the call;
  - ADR 0093: inside an Agent's task, a handler navigation's `site_preferences_refused` is a failed action given back to the model, not raised from `page.agent`.
- **Answers ADR 0013 and ADR 0081.** The built-in Agent's SDK form is `page.agent`, and its steps are a lossless sequence that starts at the task's first step.
- **Error codes.** `agent_tool_failed`, `agent_tool_invalid` and `success_check_threw` join `goodfetch-protocol` in the `program` group, through `xtask codegen`'s naming checks (ADR 0079).
- **`GLOSSARY.md`** gains **Agent tool**, and **Grounding** names an Agent tool's result as a source.
- **Area 9 (AI and agent)** builds the loop, its built-in actions with `note`, the stall rule, Grounding of the output, the task's Plan entry, its replay and the hand-over to the loop on Drift, and `agent.domains`' enforcement. **Area 11 (Client surfaces)** builds `page.agent`, the task handle per language, `AgentResult`, `AgentStep`, Agent tools and the error classes. **Area 5 (Site access)** builds `session.recovery.agent`. Each lists this ADR.
- **ADR 0037's matrix:** AI-05 (the agent loop) and V3-04 (the built-in agent loop) cite this ADR, as do the pains `agent-deterministic-replay`, `silent-false-success`, `agent-unreliable-drift`, `agent-human-in-loop` and `maint-site-drift`.
- **Tests.** TESTING.md's Agent conformance cases run in every SDK against the Scripted engine and the Scripted model (ADRs 0065, 0069).
- **Other tickets of the map.**
  - [The Run in code](https://github.com/goodfetch/goodfetch/issues/478) (ADR 0102) gives the Run's report, where the Run's Verdict and `PlanReport` sit, and reading the Run trace from code.
  - [Effects a program declares in code](https://github.com/goodfetch/goodfetch/issues/500) decides whether an Agent tool takes a function that tells whether its Effect already happened.
    > Amended by [ADR 0106](0106-a-program-declares-an-effect-as-a-keyed-block-whose-output-the-effect-journal-keeps-judged-by-what-the-engine-sent.md): it takes none; a tool that is not read-only may be `repeatable` instead, and its call carries an `idempotencyKey`.
  - [Mapping the catalog to each language and surface](https://github.com/goodfetch/goodfetch/issues/471) spells the Agent's CLI command, MCP tool and profiles, and how those surfaces state a Success check inside the task.
    > Amended by [ADR 0099](0099-every-surface-spells-each-catalog-name-by-its-own-rule-shapes-come-from-declared-kinds-and-call-is-the-way-down.md): it names the MCP tool `page_agent` and any CLI command `page agent`, and gives the profiles and the in-task Success check to area 11's spec.
  - [Testing a user's crawl or automation](https://github.com/goodfetch/goodfetch/issues/482) replays a task's Plan entry in tests.
    > Amended by [ADR 0105](0105-a-programs-tests-replay-a-response-recording-in-a-test-engine-that-reaches-no-site.md): a test starts its Run with the Plan and `plan.frozen`, with a Response recording for the pages, so a replayed task makes neither a request nor a model call.
  - [API sketch: type-checked stubs and golden scenarios](https://github.com/goodfetch/goodfetch/issues/483) writes golden scenario 11's programs.
  - The map's fog item "A call scope for Limits" gets nothing from scenario 11, whose whole program is one task in its own Run; scenario 08's model cap is [Capping a crawl's model use without stopping its Run](https://github.com/goodfetch/goodfetch/issues/513)'s (ADR 0107).
- **The `prototype/sdk-api` sketch** holds these signatures in its `agent` files for each language. The pain coverage matrix closes P04's Agent part as API.
