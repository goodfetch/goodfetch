# 08. Extract with AI inside a crawl, learning once per page type

## Task statement

> Crawl the first three categories in the side list of https://books.toscrape.com/ and every book they list, following their "next" pages. Extract each book's title, price, number in stock and star rating with a language model instead of hand-written selectors. The model is asked once per page type: later books of the same layout are extracted from what was learned, with no model call, unless a page no longer matches what was learned. Stop all model use once the crawl has spent a set amount (for example 0.50 USD), let the crawl finish without the model, and report which books were left unextracted and why. Report the tokens and cost spent.

## Target

`books.toscrape.com`: category pages and book pages, which share one layout per page type (checked 2026-10-14).

## Leader

Crawlee 3.18.2 (TypeScript): `StagehandCrawler`, on Stagehand 3. Crawlee for Python 1.10.4: the experimental `PydanticAiCrawler` with `PydanticAiSelectorExtractor`, which learns selectors once and replays them.

## Pains

P20 AI on a crawler's page, P03 replay without the model, P42 budgets and cost, P19 structured output; keeps K02 generate once, then own the deterministic artifact.

## Failure points

1. What was learned no longer matches a page with a different layout.
2. The budget runs out part-way: the rest of the crawl must be explicit about what it skipped.
3. The model returns invalid data for one book.
4. The model provider is unreachable.
5. Two handlers learn the same page type at the same time.
6. Cost or usage is not reported.

## Setup

Written by #483.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Crawlee with Stagehand (TypeScript) | | | | |
| Crawlee for Python with PydanticAI | | | | |
| Goodfetch TypeScript | | | | |
| Goodfetch Python | | | | |
| Goodfetch Rust | | | | |

## After the sketch

A new Journey on the SDK surfaces; Proving ground families: AI, extraction, and the crawl families, served by the Fake model.
