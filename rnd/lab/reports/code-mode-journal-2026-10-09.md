# Code mode with an effect journal (D-agent-interface), October 9, 2026

The first arena ([report](interface-arena-2026-10-09.md)) blocked code mode on the correctness gate. When GPT-5.6 Luna forgot `page.done()` and was told so, it rewrote the whole program. The corrected version replayed inputs and clicks on a page that had already received them: consent checkbox unticked, booking submitted twice.

## The guard

`code-mode-journal` runs the same program, but each effect (`fill`, `select`, `click`) is written to a journal. A correction is replayed against this journal from the start:
- an action identical to the one in the journal at the same position is skipped and reported as "already applied";
- a different action, while a click from the journal is still ahead of it, stops the program with `E_EFFECT_DIVERGENCE`. A click already sent cannot be reconciled. The stop holds even if the program catches the error.
- `fill` and `select` set a value and are idempotent: with no upcoming click in the journal, a divergence on them resumes live.

The correction message lists the actions already received by the page. An integration test with a scripted model reproduces the arena case: without the journal the booking is refuted, with the journal the five actions are skipped and the task is verified.

## Run

`interface-arena-journal-2026-10-09T15-11-59-464Z-f3de1a`, `local-dev`, committed sources. It covers 5 interfaces, 3 real models via the Vercel AI gateway, 3 page variants and 5 blocks of 3 tasks, i.e. 675 tasks for $0.42 in total.

| Interface | Model | Verified tasks | False successes | Unfinished | Turns per verified task | Tokens per verified task | $ per 1,000 verified tasks |
| --- | --- | --- | --- | --- | --- | --- | --- |
| code mode | `anthropic/claude-haiku-5.5` | 45/45 | 0 | 0 | 1.0 | 716 | 0.16 |
| code mode | `google/gemini-3.1-flash-lite` | 45/45 | 0 | 0 | 1.0 | 363 | 0.15 |
| code mode | `openai/gpt-5.6-luna` | 28/45 | **17** | 0 | 2.2 | 1057 | 0.52 |
| code mode + journal | `anthropic/claude-haiku-5.5` | 45/45 | 0 | 0 | 1.0 | 718 | 0.16 |
| code mode + journal | `google/gemini-3.1-flash-lite` | 45/45 | 0 | 0 | 1.0 | 363 | 0.15 |
| code mode + journal | `openai/gpt-5.6-luna` | **45/45** | **0** | 0 | 1.4 | 772 | 0.42 |
| MCP | `anthropic/claude-haiku-5.5` | 45/45 | 0 | 0 | 5.7 | 11913 | 1.66 |
| MCP | `google/gemini-3.1-flash-lite` | 45/45 | 0 | 0 | 5.7 | 2379 | 0.70 |
| MCP | `openai/gpt-5.6-luna` | 45/45 | 0 | 0 | 3.9 | 2113 | 0.58 |
| function tools (sdk) | `anthropic/claude-haiku-5.5` | 44/45 | 0 | 1 | 5.2 | 10475 | 1.45 |
| function tools (sdk) | `google/gemini-3.1-flash-lite` | 45/45 | 0 | 0 | 5.7 | 2359 | 0.71 |
| function tools (sdk) | `openai/gpt-5.6-luna` | 45/45 | 0 | 0 | 3.9 | 2125 | 0.58 |
| CLI | `anthropic/claude-haiku-5.5` | 27/45 | 0 | 18 | 9.6 | 13447 | 2.01 |
| CLI | `google/gemini-3.1-flash-lite` | 45/45 | 0 | 0 | 4.3 | 1644 | 0.58 |
| CLI | `openai/gpt-5.6-luna` | 45/45 | 0 | 0 | 4.3 | 2014 | 0.59 |

## Reading

- **The journal removes the flaw at no cost to the models that do not trigger it.** GPT-5.6 Luna's 17 false successes in code mode drop to 0. Its corrections are skipped instead of being replayed, which costs it 1.4 turns per task instead of 2.2. Claude Haiku 5.5 and Gemini Flash Lite write a complete program on the first try: their figures do not move.
- **Code mode with the journal is the cheapest interface and the only single-turn one.** It requires 3 to 15 times fewer tokens than function tools and 4 to 10 times fewer dollars, for the same correctness.
- The findings of the first arena hold with 5 blocks. MCP and function tools cost the same. The CLI depends on the model: Claude Haiku leaves 18 tasks out of 45 without declaring them finished.
- No model clicked the destructive control, whatever the interface.

## Mechanical verdict

`pnpm lab verdict D-agent-interface`, in model turns per verified task, against MCP. Since turns and tokens are counted by the gateway, the decision is made in `local-dev`.

| Option | Booking page | Destructive decoy | Price reading |
| --- | --- | --- | --- |
| function tools (sdk) | equivalent (0.95) | equivalent (1.01) | equivalent (0.98) |
| CLI | `insufficient-n` (1.09) | `insufficient-n` (1.05) | **worse** (1.43) |
| code mode | blocked by the contract | blocked | blocked |
| code mode + journal | `insufficient-n` (0.20) | `insufficient-n` (0.20) | **better** (0.34) |

Journaled code mode requires 3 to 5 times fewer turns than MCP. On the booking tasks, the rule calls for 93 to 99 blocks, because GPT-5.6 Luna adds a correction there in some blocks and not in others, which disperses the ratios. The gap is not in question: the interval stays between 0.16 and 0.26.

## What this says for Collie's interface

1. **Code mode is offered, and its effects are journaled by default.** It is the same rule as the AI layer's compiled replay: a correction executes against a known state and never resends an effect already received by the site.
2. The journal is positional and refuses to reconcile a click. This is deliberately strict: a non-idempotent action already sent calls for a review (`effect_unknown`), not a guess.
3. MCP and function tools remain two wrappers of the same operation catalog. The CLI serves humans and scripts, not agents first.

## Limitations

- Short tasks on a single page. A long journey multiplies the opportunities for divergence; the journal remains to be measured on several pages and noisy forms.
- Durations outside the verdict: they include model latency, on a `local-dev` host.
