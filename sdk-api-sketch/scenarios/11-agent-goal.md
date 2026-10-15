# 11. Reach a goal from one instruction, with limits and a verdict

## Task statement

> On https://books.toscrape.com/, from one instruction, "find the cheapest book rated five stars in the Travel category and give its title and price", reach the answer without writing the steps yourself. Cap the work at 25 steps and 0.20 USD. Return the title and the price as typed fields, together with how the work ended: stopped by a limit, reported as done by the model, or verified by a condition the program states (the page reached is a book page in the Travel category with a five-star rating). Keep the steps taken so that they can be repeated without a model. Stay on books.toscrape.com, whatever the pages suggest. Nothing about the task may be sent to any third party other than the chosen model's provider.

## Target

`books.toscrape.com`: the Travel category and its book pages (checked 2026-10-14).

## Leader

Browser Use 0.13.11 (Python): `Agent(task=..., llm=...)` and `run()`.

## Pains

P04 false success, P42 budgets and cost, P25 lean observations, P19 model breadth, P13 telemetry, P15 domain policy, P03 replay without the model; keeps K08 any model and K10 token-efficient observations.

## Failure points

1. The model reports success with a wrong answer.
2. A step or cost limit is reached: the partial answer and the reason are returned.
3. The loop repeats itself without progress.
4. The model provider is unreachable.
5. A page's observation is too large for the model.
6. Task text or addresses leave the machine to a party other than the model's provider.
7. A page leads the model off the site.

## Setup

Written by #483.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Browser Use (Python) | | | | |
| Goodfetch TypeScript | | | | |
| Goodfetch Python | | | | |
| Goodfetch Rust | | | | |

## After the sketch

A new Journey on the SDK surfaces; Proving ground family: AI, served by the Fake model. [The Agent in Collie's SDK API](https://github.com/goodfetch/automan/issues/474) answers it with `page.agent(task)` (ADR 0100): a step cap of its own, the Run's price Limit, the program's Success check, `agent.domains`, and the task kept as one Plan entry; its snippets are `snippets/agent.ts`, `snippet_agent.py` and `snippet_agent.rs`.
