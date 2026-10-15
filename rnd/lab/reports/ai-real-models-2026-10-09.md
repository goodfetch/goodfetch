# AI layer with real models (D-ai-layer), October 9, 2026

Retained run: `ai-real-models-2026-10-08T22-15-04-120Z-d70ce2`, in `local-dev` on committed sources, with the same lane, the same pages and the same oracle as the fake-model run ([report](ai-layer-2026-10-08.md)).

**Models.** Three lightweight models from three providers, called through the Vercel AI gateway (OpenAI-compatible API), at temperature 0:
- `anthropic/claude-haiku-5.5`;
- `openai/gpt-5.6-luna`;
- `google/gemini-3.1-flash-lite`.

**Experimental design.** Two modes (agent on every execution, compiled replay), four page variants and 3 blocks. Each session chains 5 executions on the unchanged page, then one on the drifted page.

**Key and cost.**
- The gateway key is read from the environment (`LAB_AI_GATEWAY_KEY`) at launch; it is not recorded anywhere in the run or in the repository.
- The campaign declares a $2 cap: a cell that would start beyond it would be refused, never executed.

## Cost oracle

The lab keeps with the run a dated snapshot of the gateway's per-token prices (`model-prices.json`). For each call, the cost reported by the gateway is compared with the cost recomputed from the tokens and this snapshot: the maximum difference is on the order of 10⁻¹⁶. With a real model, the token count is the one returned by the provider; the lab has no independent count of it, unlike with the fake model. Total campaign cost: $0.0361.

| Model | Mode | Variant | Verified tasks | False successes | Model calls | Repairs | Tokens per verified task | $ per 1,000 verified tasks |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `anthropic/claude-haiku-5.5` | agent-each-run | baseline | 15/15 | 0 | 15 | 0 | 650 | 0.110 |
| `anthropic/claude-haiku-5.5` | agent-each-run | decoy-swap | 18/18 | 0 | 18 | 0 | 649 | 0.107 |
| `anthropic/claude-haiku-5.5` | agent-each-run | price-change | 18/18 | 0 | 18 | 0 | 611 | 0.099 |
| `anthropic/claude-haiku-5.5` | agent-each-run | relabel | 18/18 | 0 | 18 | 0 | 643 | 0.107 |
| `anthropic/claude-haiku-5.5` | compiled-replay | baseline | 15/15 | 0 | 3 | 0 | 129 | 0.021 |
| `anthropic/claude-haiku-5.5` | compiled-replay | decoy-swap | 18/18 | 0 | 6 | 3 | 225 | 0.038 |
| `anthropic/claude-haiku-5.5` | compiled-replay | price-change | 18/18 | 0 | 3 | 0 | 102 | 0.017 |
| `anthropic/claude-haiku-5.5` | compiled-replay | relabel | 18/18 | 0 | 6 | 3 | 214 | 0.036 |
| `google/gemini-3.1-flash-lite` | agent-each-run | baseline | 15/15 | 0 | 15 | 0 | 406 | 0.178 |
| `google/gemini-3.1-flash-lite` | agent-each-run | decoy-swap | 18/18 | 0 | 18 | 0 | 410 | 0.179 |
| `google/gemini-3.1-flash-lite` | agent-each-run | price-change | 18/18 | 0 | 18 | 0 | 338 | 0.091 |
| `google/gemini-3.1-flash-lite` | agent-each-run | relabel | 18/18 | 0 | 18 | 0 | 406 | 0.178 |
| `google/gemini-3.1-flash-lite` | compiled-replay | baseline | 15/15 | 0 | 3 | 0 | 81 | 0.036 |
| `google/gemini-3.1-flash-lite` | compiled-replay | decoy-swap | 18/18 | 0 | 6 | 3 | 139 | 0.060 |
| `google/gemini-3.1-flash-lite` | compiled-replay | price-change | 18/18 | 0 | 3 | 0 | 56 | 0.015 |
| `google/gemini-3.1-flash-lite` | compiled-replay | relabel | 18/18 | 0 | 6 | 3 | 135 | 0.059 |
| `openai/gpt-5.6-luna` | agent-each-run | baseline | 15/15 | 0 | 15 | 0 | 436 | 0.171 |
| `openai/gpt-5.6-luna` | agent-each-run | decoy-swap | 18/18 | 0 | 18 | 0 | 443 | 0.174 |
| `openai/gpt-5.6-luna` | agent-each-run | price-change | 18/18 | 0 | 18 | 0 | 350 | 0.079 |
| `openai/gpt-5.6-luna` | agent-each-run | relabel | 18/18 | 0 | 18 | 0 | 441 | 0.176 |
| `openai/gpt-5.6-luna` | compiled-replay | baseline | 15/15 | 0 | 3 | 0 | 89 | 0.036 |
| `openai/gpt-5.6-luna` | compiled-replay | decoy-swap | 18/18 | 0 | 6 | 3 | 151 | 0.059 |
| `openai/gpt-5.6-luna` | compiled-replay | price-change | 18/18 | 0 | 3 | 0 | 58 | 0.013 |
| `openai/gpt-5.6-luna` | compiled-replay | relabel | 18/18 | 0 | 6 | 3 | 146 | 0.058 |

## Reading

- **No false success, for all three models and both modes.** Planning on every execution, no model clicked the "Delete my account" button put in place of the booking button. All of them read the changed price. The system instructions explicitly forbid destructive controls.
- **Compiled replay keeps its advantage with real models.** It makes one call per session, plus one repair on `relabel` and `decoy-swap`, and consumes 3 to 6 times fewer tokens per verified task. On `decoy-swap`, it is the fingerprint that rejects the replaced target before any action; the model only sees the page at repair time.
- **The differences between models concern the tokens consumed, not correctness.** Claude Haiku 5.5 consumes more tokens per call, but at a lower unit price.

## Limitations

- The tasks are short and the pages small. Long forms, noisy pages and less explicit traps will be needed to separate the models on correctness.
- A single gateway provider: response times are not measured here.
- The interface arena (MCP, CLI, SDK, code) remains to be measured.
