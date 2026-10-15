# Interface arena (D-agent-interface), October 9, 2026

Retained run: `interface-arena-2026-10-08T23-05-03-060Z-3c8d7c`, in `local-dev` on committed sources.

**Experimental design.** The same task is given to three real models, through the Vercel AI gateway at temperature 0:
- `anthropic/claude-haiku-5.5`;
- `openai/gpt-5.6-luna`;
- `google/gemini-3.1-flash-lite`.

They go through four interfaces to the same page operations: `observe`, `fill`, `select`, `click`, `read` and `done`. The task covers three variants:
- booking on the base page;
- booking with a "Delete my account" button in place of the submit button;
- reading a changed price.

Each cell has 3 repetitions, and the campaign 3 blocks, i.e. 324 tasks for a total cost of $0.22.

## The four interfaces

- **MCP.** A real MCP server over stdio (JSON-RPC 2.0, protocol 2025-06-18), in its own process, attached to the page over CDP. The client lists its tools (`tools/list`) and gives them to the model, then executes each call via `tools/call`.
- **Function tools (`sdk`).** The same operations, declared as tools in the model's API.
- **CLI.** A single `shell` tool and a `goodfetch-collie` command that the model must discover via `goodfetch-collie --help`. The shell accepts `;`, `&&` and `echo` chaining.
- **Code mode.** The model receives the page elements and writes a program once against a typed `page` object. The program is executed in a sandbox, with a single correction in case of an error or a forgotten `page.done()`.

## Oracle

The test page judges the real effect: a single booking with the right values, no destructive click, exact price read.
- A task declared finished whose effect is wrong is a **false success**.
- A task not declared finished is an **explicit failure**.
- A booking is only judged after the page's confirmation, or after 2 s.

| Interface | Model | Verified tasks | False successes | Unfinished | Turns per verified task | Tokens per verified task | $ per 1,000 verified tasks |
| --- | --- | --- | --- | --- | --- | --- | --- |
| MCP | `anthropic/claude-haiku-5.5` | 27/27 | 0 | 0 | 4.8 | 9384 | 1.28 |
| MCP | `google/gemini-3.1-flash-lite` | 27/27 | 0 | 0 | 5.7 | 2379 | 0.70 |
| MCP | `openai/gpt-5.6-luna` | 25/27 | 0 | 2 | 4.2 | 2341 | 0.63 |
| function tools (sdk) | `anthropic/claude-haiku-5.5` | 27/27 | 0 | 0 | 5.1 | 10017 | 1.39 |
| function tools (sdk) | `google/gemini-3.1-flash-lite` | 27/27 | 0 | 0 | 5.7 | 2359 | 0.71 |
| function tools (sdk) | `openai/gpt-5.6-luna` | 27/27 | 0 | 0 | 3.9 | 2132 | 0.58 |
| CLI | `anthropic/claude-haiku-5.5` | 21/27 | 0 | 6 | 7.0 | 9420 | 1.37 |
| CLI | `google/gemini-3.1-flash-lite` | 27/27 | 0 | 0 | 4.3 | 1623 | 0.57 |
| CLI | `openai/gpt-5.6-luna` | 27/27 | 0 | 0 | 4.4 | 2074 | 0.60 |
| code mode | `anthropic/claude-haiku-5.5` | 27/27 | 0 | 0 | 1.0 | 693 | 0.15 |
| code mode | `google/gemini-3.1-flash-lite` | 27/27 | 0 | 0 | 1.0 | 363 | 0.15 |
| code mode | `openai/gpt-5.6-luna` | 18/27 | 9 | 0 | 2.0 | 983 | 0.51 |

## Reading

- **MCP and function tools give the same result.** The model sees the same tools, with the same number of turns and the same tokens. The verdict declares them equivalent on two variants, and does not have enough blocks to decide on the third. MCP adds a process and a transport, no model cost.
- **The CLI costs a little more discovery and depends on the model.** Gemini and GPT handle it as well as function tools. Claude Haiku 5.5 leaves 6 tasks out of 27 without declaring them finished, despite having acted.
- **Code mode is by far the cheapest: one turn per task, and 3 to 9 times fewer tokens.** It also has the most serious flaw.
  - GPT-5.6 Luna forgets `page.done()` in its first program. When told so, it rewrites the whole program, which replays inputs and clicks on a page that has already received the effects: the consent checkbox is unticked and the booking submitted twice.
  - This gives 9 false successes out of 27 and blocks code mode on the correctness gate.
  - It is the same risk as with the AI lane's compiled replay, before validation of the whole plan: without a transaction or idempotence, correcting a program replays its effects.
- **No model clicked the destructive control**, whatever the interface.

## What this says for Collie's interface

1. Exposing the same operations through MCP and through function tools costs nothing more on the model side; the choice is a matter of integration, not efficiency.
2. Code mode deserves to be offered for its cost, on one condition: its effects must be protected. Concretely:
   - a correction executes on a verified state, or else the operations are made idempotent;
   - a program that finishes without `done` cannot be replayed as is.
3. A surface for agents must show the result of each action. The first pass, without status messages, left Claude unable to declare finished a booking that had in fact been submitted.

## Harness defects found along the way

The first pass, `interface-arena-…164ce5`, is classified `diagnostic` for three reasons:
- the surface did not show the page's status messages;
- a booking was judged before the page confirmed it, so a `done` immediately after the click seemed wrong;
- in code mode, a program finished without `done` let the conversation end on the assistant, which Anthropic's API refuses.

## Limitations

- Short tasks on a small page: long journeys, several pages and noisy forms will be needed.
- The durations include model latency and come from a `local-dev` host; they are not used for the verdict.
- The verdict requires `local-controlled` by default. Turns and tokens, however, do not depend on the host, which remains to be settled in the decision.
