# Collie's Agent is one bounded call on a page, judged by the program's Success check and replayed whole from the Plan

A program hands Collie's built-in Agent one objective with `page.agent(task, options)`. The Agent is ADR 0013's thin, journaled loop over the Operation catalog, run as one call that ends in ADR 0079's three separate fields: how it ended, its Verdict and what the model claimed. The program states the Success check with Playwright's `expect` (ADR 0091). A step cap, a stall rule and the task's own timeout stop the task with a reason and its partial answer. The steps are kept as one entry of the Run's Plan, replayed with no model call and repaired once on Drift. By default the task stays on the site it started on. The leaders split on this: Stagehand removed `agent()` in v4 and Playwright removed its hidden `page.agent()` after one release, while Browser Use, Skyvern and Notte keep a loop whose false successes and silent hangs are among the research's most-reported pains ([AI browser automation](https://github.com/goodfetch/automan/blob/research/consumer-api/rnd/research/research_notes/Consumer%20API%20of%20competitors/ai-browser-automation.md) §4.1 item 6, §4.2 and §6 pains 1, 2, 8 and 13). The loops' exact defaults were checked at tagged releases in [Research: agent loop APIs](https://github.com/goodfetch/automan/blob/research/sdk-agent-loops/rnd/research/2026-10-15-sdk-agent-loops.md). Decided in [The Agent in Collie's SDK API](https://github.com/goodfetch/automan/issues/474), on the map [The SDK API of Collie and Foxhound](https://github.com/goodfetch/automan/issues/461), every answer decided by the agent on the owner's delegation, with its [grilling record](https://github.com/goodfetch/automan/issues/474#issuecomment-6181861259). Principles are cited by their IDs in [ADR 0077](0077-the-sdk-api-follows-eight-guarantees-and-four-ranked-shape-rules.md). The page is [ADR 0085](https://github.com/goodfetch/automan/pull/501)'s, the AI verbs, Plans and `expect` are [ADR 0091](https://github.com/goodfetch/automan/pull/510)'s, Sessions and secrets are [ADR 0092](https://github.com/goodfetch/automan/pull/509)'s, models and Usage are [ADR 0101](https://github.com/goodfetch/automan/pull/515)'s, a crawl handler's page is [ADR 0093](https://github.com/goodfetch/automan/pull/512)'s and Politeness is [ADR 0086](https://github.com/goodfetch/automan/pull/497)'s.

## Decision

### One call on a page

- **The SDK offers the built-in Agent** (G1, G2, S2). ADR 0013 rejected "no built-in agent" and gives it "the user's objective tasks", and ADR 0037 decided AI-05 and V3-04. The research found no GitHub complaint about Stagehand's removal, only one HN question, while the embedded loops are where the false-success and hang reports are (AI §4.2). The Verdict and the bounds below answer those. Each call runs one task to its end, and nothing carries into the next call: a remembered conversation that a follow-up task silently continues, as Browser Use's `add_new_task`, would be hidden state (G2).
- **`page.agent(task, options)`, on the page only** (G2, S1, S3, T). The task starts on a page the program holds, so a Borrowed page (`run.borrow(page)`, ADR 0078), a crawl handler's page and a popup get it, as they get ADR 0091's verbs. Skyvern's `page.agent.run_task(prompt)` works the same way and "returns control on the same page".
  - **Not an agent object configured, then run**, as Browser Use's `Agent(…).run()`, Notte's `Agent(session).run(task=…)` and Stagehand v3's `agent({…}).execute(…)`. That would be one more value and lifecycle (S2, T), and `run` already names a Run in Goodfetch (S3). A model that writes Stagehand's form meets a type error first (G3).
  - **Not on a locator,** whose scope would end at the first navigation, and **not on the Run,** which would deny the program logging in or navigating before it hands over.
- **The page and its tabs.** The task acts on its page and on the tabs it opens, never on the program's other pages. The whole task is one gesture under ADR 0085's per-page order: the program's later gestures on that page wait for it, while reads and waits run. When the task ends, the tabs it opened are closed, except the page it ended on, which the result returns (G4). On a crawl handler's page the task is an acting AI verb, so under `fetch.tier: auto` it escalates to the browser, and under a forced tier it fails with `browser_required` (ADR 0093).
- **What the call takes** (G3, S2). `page.agent(task, { schema, successCheck, maxSteps, domains, tools, variables, model, timeout, signal })`:
  - `task` is the objective in words. Its `%name%` placeholders follow ADR 0091: `variables` are plain values, as `act` takes them, and a placeholder that no variable fills is a Secret placeholder bound by `secrets.<name>` (ADR 0092). The model, the Plan and the logs see only `%name%`.
  - `schema` is the output's schema in ADR 0082's forms. Without one, the output is nothing, and the model's message is in `reported`.
  - `maxSteps`, `domains`, `model` and `timeout` are per-call arguments, each that setting at the run layer (ADR 0083): `agent.max_steps`, `agent.domains`, `ai.model` and `timeout.agent`. There is no Agent-only model key, since `model` already picks a model for one task (ADR 0101, S2).
  - `agent.max_stalled_steps` and `agent.human_pause` are settings only. The system prompt, vision and the Decision provider stay settings of their scopes (ADRs 0013, 0083, 0101).
  - Python spells the options `success_check`, `max_steps` and so on, as keyword-only arguments. Rust spells them as builder methods.

### The Success check

- **The program states it with `expect`** (G1, G6, S3, T): `successCheck: async (page) => { await expect(page.locator(".breadcrumb")).toContainText("Travel"); … }`. Each `expect` in it is a Success check the engine evaluates live and records (ADR 0091). The function runs where the program's code runs (ADR 0081), so it can state any condition Playwright's assertions state, over several elements. A declarative value (`[check.url(…)]`) would name Playwright's conditions anew, which ADR 0091 rejected for single checks (S3). A model-judged check, as Browser Use's judge and Notte's `CompletionValidator`, is ruled out by ADR 0013. The function is not data, so the CLI and MCP state their checks through the catalog's `expect` operation instead. A Plan never holds it: ADR 0009 says "a Success check is never cached".
- **When it runs.** Each time the model reports done, on the page the task is on.
  - When it passes, the task finishes.
  - When an `expect` in it fails, the failure goes back to the model as the answer to its "done", and the loop goes on within its bounds. The failure is the check as rendered, with what was observed. Notte's validator and Pydantic AI's `ModelRetry` feed back a model's judgment the same way; here it is a condition.
  - It does not run when the model gives up or the task stops early.
- **The Verdict reads its latest evaluation** (ADR 0029). The task is `verified` when that evaluation passed and every other check it relies on holds. It is `rejected` when that evaluation failed. It is `needs-review` when the check never ran or the task declared none (ADR 0013).
- **An exception from the function other than `success_check_failed`** is the program's bug, never a rejection: the call raises `success_check_threw`, a `ProgramError` chaining it (G1).

### The result

- **`AgentResult<T>` is ADR 0079's `AiWorkResult<T>`** (`ended`, `verdict`, `reported`, `output`, `partial`, `requireVerified()`), plus three fields (G1, G6, S1):
  - `steps`, every step of the task;
  - `usage`, ADR 0101's `ModelUsage` for the whole task;
  - `page`, the page where the task ended.
- **`output`** is a complete `T`: the model's final answer, checked against the schema and the program's validators (ADR 0082) within ADR 0012's bounded retries. An answer that never passes raises `model_output_invalid` (ADR 0101).
- **Grounding holds the output's values** (ADR 0013). Each value must appear in a page the task observed, or follow from a declared transform. A value that does not is fed back once through ADR 0012's retry; if it stays ungrounded, the output is returned with `verdict: needs-review` naming the field, since a valid `T` the checks cannot prove is a Verdict's business (ADR 0029). Grounding also records where each value was found, which is how a Plan reads it again.
- **`reported`** is the model's claim, `done` or `gave_up` with its message. It is absent when the task stopped before the model claimed anything.
- **`ended`** is `finished` when the model reported done or gave up, or `stopped` with its reason (below).
- **`partial`, from a `record` action** (G1, S1). The Agent has a built-in action that records what it has found so far against the schema. `partial` holds the latest recorded or answered value of each top-level field, or of each element for a list output, that is valid against its own part of the schema and Grounded, and nothing else. It is set only when `output` is not.
  - TypeScript types it `PartialOf<T>`: `Partial<T>` for an object, the elements for a list.
  - Python gives a mapping or a list of validated values.
  - Rust gives `PartialOutput`, with typed accessors.

  Stagehand v3 returned `completed: false` with the actions taken, and Notte `success=False` with its answer. The AI SDK's `.output` throws when the last step ended on tool calls.

### Bounds

- **A step cap, not a Limit** (G5, S2, S3). `agent.max_steps` is **20** by default, the value the AI SDK's `ToolLoopAgent`, Stagehand v3, Midscene and Notte converged on; `maxSteps` sets it for one task. The others range from Playwright 1.58's 10 actions and OpenAI Agents' 10 turns to Browser Use's 500.
  - A step is one model decision and the actions it takes, or one step replayed from the Plan. A Human pause is no step (ADR 0092).
  - Reaching the cap stops only the task, and the Run goes on, whereas a Limit stops the Run (ADR 0080). That is why ADR 0080's rejected step Limit is not this cap.
- **A stalled loop stops** (G1, G6). `agent.max_stalled_steps` is **3** by default, as Notte's `max_consecutive_failures`. A step stalls when its actions failed, or when it repeated earlier actions on targets whose page had not changed since. Browser Use's silent "Step 1" loops are its most-commented issues (#839, #1020).
- **The task's own timeout.** `timeout.agent` is **10 minutes** by default, and `timeout` sets it for one task. It nests the actions' timeouts and ADR 0101's `timeout.model`. A task that reaches it stops at the end of its current step, and time in a Human pause does not count (ADR 0080).
- **Every early end is a value** (G1, S2). As ADR 0079 makes AI work stopped by a Limit, the result is `ended: { stopped, reason }`, with `steps` and `partial` kept. A program reads every early end in one place. The reasons:
  - `steps`: the step cap, naming the setting and its layer;
  - `no_progress`: the stall rule, naming the setting and its layer;
  - `timeout`: the task's own timeout or the Run's, naming the setting;
  - `limit`: a Limit by name, after which the Run stops too (ADR 0080);
  - `abort`: someone other than the caller stopped the task, such as a person in the Console, another Client, or the program closing the page or the Run.

  The leaders split here: OpenAI Agents, Pydantic AI, Midscene and Playwright 1.58 raise at their cap, while the AI SDK, Stagehand v3, Notte and Browser Use return. The caller's own cancellation keeps its language's form (ADR 0080).

### Steps as they happen

- **A lossless sequence, from the task's first step** (G1, G6). ADR 0081 makes an Agent's steps "a sequence read to its end". Every step is a record a program may log or show, so none is dropped, and an unread open iterator slows the task (ADR 0081). Each `steps()` call is a new reader that starts at step 1 whenever it is called, so nothing slips past a late reader, as ADR 0080's waiters arm when called. The iterator ends when the task ends.
- **One call returns a task handle,** shaped per language (S1 over S4, T). Every leader that streams a model loop gives one value to iterate and then ask for the final result: OpenAI Agents' `StreamedRunResult` and `run_streamed()`, Anthropic's `messages.stream()` with `finalMessage()`, the AI SDK's `stream()`.
  - **TypeScript:** `page.agent(…)` starts the task and returns an `AgentTask<T>`, a `Promise<AgentResult<T>>` that also has `steps()`, as openai-node's `APIPromise` adds methods to a promise. `await page.agent(…)` is the whole program in the common case.
  - **Python:** `page.agent(…)` returns an `AgentTask`, or an `AsyncAgentTask` in the async API, with `result()`, `steps()` and a `with` block (`async with`), as Anthropic's and OpenAI's stream helpers do. It starts at the first of `result()`, `steps()` or entering the block. Leaving the block before the task ends cancels it. The sync and async APIs keep one shape: `await page.agent(…).result()` and `page.agent(…).result()`.
  - **Rust:** `page.agent(task)` is a builder that `.await`s to the result, as ADR 0080's calls do. `.start().await?` gives an `AgentTask` with `steps()`, an `EventStream`, and `result()`.
- **An `AgentStep`** carries:
  - its number;
  - its `origin`, `model`, `plan` or `repair`, in ADR 0091's words;
  - the model's own message, a signal, empty for a replayed step;
  - its actions, each with the catalog operation, its target as a `Locator`, the call rendered in the caller's language as `code` (ADR 0091's renderer), and its outcome: done, failed with its error, or already in the Effect journal;
  - the page's URL after the step;
  - whether it stalled;
  - its `ModelUsage`.

### Agent tools

- **The program adds its own actions** (G1, G3, S2). An **Agent tool** is a function of the program that the model may call as an action during the task, such as a lookup in the program's database. Each language spells it its own way: `agentTool({ name, description, input, readOnly, category, run })` in TypeScript, the `@agent_tool` decorator in Python, and `AgentTool::new` in Rust.
  - Its input schema takes ADR 0082's forms and is checked before it runs.
  - It runs where the program's code runs (ADR 0081), with the input and the task's current page.
  - It returns JSON the model reads, bounded by `observation.max_size` (ADR 0091).

  OpenAI Agents' `@function_tool`, Pydantic AI's `@agent.tool`, the AI SDK's `tool()` and Browser Use's `@tools.action` are the forms models know. Browser Use injects parameters by name, and a wrong name fails silently.
- **An Effect unless `readOnly`** (G5). As ADR 0011 classes a site's WebMCP tools, an Agent tool is journaled and never redone (ADR 0010). A `category` from ADR 0014's list asks for confirmation unless the run allows it.
- **An exception ends the task** with `agent_tool_failed`, a `ProgramError` chaining it, as `hook_failed` wraps a Hook's exception (ADR 0079). A bug must not read as the model's problem (G1). An answer the model should act on is a value the tool returns; in Pydantic AI too, any exception other than `ModelRetry` ends the run. A tool that is not read-only and threw leaves its Effect uncertain (ADR 0010), and the error says so.
- **Two tools with one name, or a tool named like a built-in action,** are refused with `agent_tool_invalid`, a `ProgramError`, before the task starts (G3).
- **No MCP servers as tools.** A tool's function can call any MCP client the program uses, so the SDK needs no new type for it (G7, S2).

### The Plan

- **One Plan entry per task** (G2, S2). ADR 0091 keys Plan entries per call and leaves "a Plan replayed toward a goal", which "keeps ADR 0009's whole-sequence rule", to this ticket; golden scenario 11 asks to "keep the steps taken so that they can be repeated without a model".
  - The entry lives in the Run's Plan file (ADR 0091's `plan`).
  - It is keyed by the verb, the task with its placeholders, the hash of the schema's JSON Schema and the names of the program's tools, with one variant per start page layout.
  - It holds the actions that succeeded, in order, each as ADR 0091's `act` step: its frame path, locator ladder, Target fingerprint, method and arguments with placeholders, and Effect class.
  - It holds the tool calls with their arguments.
  - Per output field, it holds the locator ladder and transform where Grounding found the value, so the output is read again with no model, as ADR 0013's compiled mode.
- **Replayed whole, under ADR 0009's rule for a whole Plan** (G1):
  - No Effect is performed until every target the replay can reach without one has been checked against its Target fingerprint.
  - Then the output is read from the page and the Success check runs, as in a live task.
  - Its steps report `origin: plan`, with no model call.

  A replayed task repeats the decisions the model made, and the Success check is what vouches for them. Browser Use's `rerun_history` still calls a model for extraction and a summary, and its beta ignores the recorded run; Skyvern and Notte turn a run into code on their servers. "Replay without the model" is AI pain 1.
- **On Drift, one Repair.** The loop goes back to the model from the step that drifted, within the task's bounds, and the steps it takes replace the rest of the entry.
  - Under `plan.frozen`, or with no model configured, the Drift raises `plan_drift`.
  - Under `plan.frozen`, a missing entry raises `plan_step_missing` (ADR 0091).
- **Promotion follows ADR 0091:** a new version is promoted after the Run finishes `verified`, with ADR 0009's review of Plans with Effects. Without `plan`, the sequence lives only in the Run trace.
- **A task graduates to code.** Every page action in `steps` carries its `code` in the caller's language, so a person can turn a task into plain page API calls. That is the research's most-liked quality: generate with AI, then own deterministic code (AI §5, like 1).

### Where the Agent may go

- **`agent.domains`, by default the registrable domain of the page the task starts on** (G5). `domains` sets it for one task. An entry matches its host and their subdomains, and `*` allows wherever the run's domain policy allows; `*` applies only when written. It narrows ADR 0014's run policy and is enforced by the engine in the same way. A navigation or request outside it is refused, and the refusal goes back to the model as a failed action. Browser Use's `allowed_domains` and Stagehand's and Skyvern's domain filters were bypassed (AI-9; Stagehand #2945, Browser Use #5099, Skyvern #8577), which is why ADR 0014 enforces policy in the engine. Golden scenario 11 says "Stay on books.toscrape.com, whatever the pages suggest".
- **No robots.txt opt-in.** ADR 0086 left "whether an Agent offers an opt-in to obey robots.txt for its own navigation" here. Declined:
  - An Agent's navigation is the program's own (ADR 0086), working for one person toward one task. Google's user-triggered fetchers generally ignore robots.txt, and OpenAI says it may not apply to user-initiated actions.
  - robots.txt is still read and recorded in Provenance, and `agent.domains` already bounds where the Agent goes.
  - On a crawl handler's page, the task's navigation is the handler's, governed by Politeness and robots.txt (ADRs 0086, 0093), and its refusals go back to the model.

  A later ticket reopens it with new evidence.
- **`agent.human_pause`, `false` by default.** When on, the model may ask a person through a Human pause (ADR 0092), with its reason shown in "Needs you" (ADR 0031). It is off by default because an unattended program would wait up to `timeout.human_pause`, 24 hours (G5).
- **Confirmations.** Effects in ADR 0014's categories still ask for confirmation unless the run allows them. A person declining ends the task with `effect_not_confirmed`, as for `act` (ADR 0091), so the model never tries another way to do what a person refused.
- **`session.recovery.agent`, `true` by default.** It switches ADR 0006's AI agent login step, the built-in Agent with the Session's Secret placeholders, as ADR 0092's `session.recovery.human_pause` switches the Human pause step. The step is skipped when no model is configured and no login Plan replays it, or under `plan.frozen` without one, and `logged_out` (ADR 0092) lists why.
- **What the model reads.** Page text is marked untrusted (ADR 0014). Observations are ADR 0091's `ai` aria snapshots, bounded by `observation.max_size`, so a page too large for the model collapses its deepest subtrees instead of failing.

### Failures

Under ADR 0079's rule, the call raises when it cannot keep its promise, and AI work that ends early is a value:

| Case | Result |
|---|---|
| A failed page action, a refused navigation, an answer that fails Grounding, a failed Success check | not raised: the step's outcome, given back to the model |
| The step cap, the stall rule, the task's timeout, a Limit reached during the task, someone else stopping it | a value: `ended: stopped` with its reason |
| A Limit already used up when the task starts | `limit_reached`, a `RefusedError` (ADR 0080) |
| The Model provider fails after ADR 0012's retries; an answer that never passes the schema | ADR 0101's codes, such as `model_unavailable` and `model_output_invalid` |
| `plan.frozen` with no entry; a Drift with no model or under `plan.frozen` | `plan_step_missing`; `plan_drift` (ADR 0091) |
| An Effect sent and left uncertain | `effect_uncertain`, a `PageError` (ADR 0091): the task ends, so the model never tries the Effect again (ADR 0010) |
| A person declined a confirmation | `effect_not_confirmed`, a `RefusedError` (ADR 0091) |
| An Agent tool threw; two tools share a name or take a built-in's | `agent_tool_failed`; `agent_tool_invalid`, each a `ProgramError` |
| The Success check function threw something other than `success_check_failed` | `success_check_threw`, a `ProgramError` |
| A placeholder that neither a variable nor a binding fills | `variables_mismatch` (ADR 0091) |

An error raised after steps were taken carries the task's `usage` (ADR 0101). Its steps are in the Run trace under the error's `callId` (G6).

### Outside agents

- **Unchanged from ADR 0011** (S2). The Agent is one more catalog operation, classed Effect, so the CLI, MCP and the Framework adapters reach it where their profiles include it.
- **An outside agent** drives the same page operations the built-in Agent uses, reads ADR 0091's `ai` aria snapshots with element references, and states its Success check through the catalog's `expect`. Its run gets the same Verdict, and no model's word makes it `verified` (ADR 0029).
- **A program running its own loop** (OpenAI Agents, the AI SDK) uses `ariaSnapshot({ mode: "ai" })`, `getByRef`, the page API and `expect` from the SDK, or ADR 0011's Framework adapters.

## Considered Options

- **No Agent in the SDK,** as Stagehand v4 and Playwright: rejected. It would reverse ADRs 0013 and 0037, which the map's Notes leave to the owner, and golden scenario 11 would need every program to write its own loop.
- **An agent object configured once, then run,** as Browser Use, Notte and Stagehand v3: rejected by S2 and S3 (`run` names a Run). **Follow-up tasks** over a remembered conversation: rejected by G2.
- **A declarative Success check value:** rejected by S3, as ADR 0091 rejected one for single checks. **A model-judged check:** rejected by ADR 0013.
- **Raising at the step cap,** as OpenAI Agents, Pydantic AI, Midscene and Playwright 1.58: rejected. AI work stopped early is a value (ADR 0079), and a raised cap would lose the partial answer scenario 11 asks for, or move it into an error's details. **Raising `deadline_exceeded` at the task's timeout, or `call_cancelled` when someone else stops it** (ADR 0080's rule for other calls): rejected for the same reason. One way to read "ended early" (S2).
- **A step Limit** in ADR 0041's ledger: rejected, as ADR 0080 rejected it. The cap bounds one task and stops only it, while a Limit stops the Run.
- **Browser Use's 500 steps,** or none: rejected by G5. **OpenAI Agents' 10 turns or Playwright's 10 actions:** the low end of the leaders' range; four of them converged on 20 (S3).
- **`partial` as the model's last answer in raw JSON:** rejected by G1, since values no check vouches for would read as found. **No `partial`:** rejected, since scenario 11 asks for one.
- **Steps only on the result,** or a step listener: rejected by ADR 0081, which makes them an iterator. **Two methods**, one for the result and one streaming, as OpenAI Agents and the Anthropic SDK: rejected by S2, since one handle serves both.
- **A tool's exception fed back to the model,** as OpenAI Agents and Browser Use: rejected by G1. A bug would hide in what the model reads, and OpenAI Agents now sends only a fixed message. **MCP servers as tools,** as Browser Use and Stagehand v3: rejected by S2 and G7.
- **Replay by position with a model fallback,** as Browser Use's `rerun_history`: rejected. It still calls a model for extraction, so it is not replay without the model.
- **No domain bound beyond the run's policy:** rejected by G5, because a model reading untrusted pages decides where to go. **A robots.txt opt-in:** declined above.

## Consequences

- **Amends ADR 0079.** AI work's stop reasons gain `steps` and `no_progress`, and `timeout` covers an Agent's own timeout as well as the Run's. `reported` is absent when the work stopped before the model claimed anything. `partial` holds the top-level fields, or the elements of a list, that are valid against their part of the schema and Grounded.
- **Amends ADR 0080.**
  - An Agent's task that reaches `timeout.agent` stops with `ended: stopped`, reason `timeout`, instead of raising `deadline_exceeded`.
  - An Agent's task stopped by someone other than the caller returns `stopped` with reason `abort` instead of raising `call_cancelled`.
  - The Agent's own step cap is `agent.max_steps`, which is not a Limit.
- **Amends ADR 0083.** `timeout.agent` joins the timeout family. `agent.max_steps`, `agent.max_stalled_steps`, `agent.domains`, `agent.human_pause` and `session.recovery.agent` join the settings schema. The call's `maxSteps`, `domains`, `model` and `timeout` are those settings at the run layer.
- **Amends ADR 0009.** An Agent's Repair is the loop resumed with the model from the step that drifted. The steps it takes replace the rest of the task's entry, checked as live steps are.
- **Amends ADR 0006.** `session.recovery.agent` switches the AI agent login step of the recovery ladder.
- **Answers ADR 0013, ADR 0080 and ADR 0081.** The built-in Agent's SDK form is `page.agent`. Its step cap is `agent.max_steps`. Its steps are a lossless sequence that starts at the task's first step.
- **Error codes.** `agent_tool_failed`, `agent_tool_invalid` and `success_check_threw` join `goodfetch-protocol` in the `program` group, through `xtask codegen`'s naming checks (ADR 0079).
- **`GLOSSARY.md`** gains **Agent tool**.
- **Area 9 (AI and agent)** builds the loop, its built-in actions with `record`, the stall rule, Grounding of the output, the task's Plan entry, its replay and Repair, and `agent.domains`' enforcement. **Area 11 (Client surfaces)** builds `page.agent`, the task handle per language, `AgentResult`, `AgentStep`, Agent tools and the error classes. **Area 5 (Site access)** builds `session.recovery.agent`. Each lists this ADR.
- **ADR 0037's matrix:** AI-05 (the agent loop) and V3-04 (the built-in agent loop) cite this ADR.
- **Tests.** TESTING.md's Agent conformance cases run in every SDK against the Scripted engine and the Scripted model (ADRs 0065, 0069).
- **Other tickets of the map.**
  - [The Run in code](https://github.com/goodfetch/automan/issues/478) gives the Run's report, where the Run's Verdict and `PlanReport` sit, and reading the Run trace from code.
  - [Effects a program declares in code](https://github.com/goodfetch/automan/issues/500) decides whether an Agent tool takes a check that tells whether its Effect already happened.
  - [Mapping the catalog to each language and surface](https://github.com/goodfetch/automan/issues/471) spells the Agent's CLI command, MCP tool and profiles.
  - [Testing a user's crawl or automation](https://github.com/goodfetch/automan/issues/482) replays a task's Plan entry in tests.
  - [API sketch: type-checked stubs and golden scenarios](https://github.com/goodfetch/automan/issues/483) writes golden scenario 11's programs.
  - The map's fog item "A call scope for Limits" gets nothing from scenario 11, whose whole program is one task in its own Run; scenario 08's model cap is [Capping a crawl's model use without stopping its Run](https://github.com/goodfetch/automan/issues/513)'s.
- **The `prototype/sdk-api` sketch** holds these signatures in its `agent` files for each language. The pain coverage matrix closes P04's Agent part as API.
