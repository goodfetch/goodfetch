# 02. Extract a typed object with a chosen model

## Task statement

> From https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html, extract the book as a typed object: its title (text), its price (a decimal number of pounds), the number of copies in stock (an integer), its star rating (one of one to five) and its description (text). Use a language model to read the page instead of hand-written selectors. The program chooses the model by name, and a model served locally must work as well as a hosted one. Print the tokens used and their cost next to the result. If a field cannot be found or does not have the expected type, say which field, instead of returning an empty or wrong value.

## Target

A product page of `books.toscrape.com`: "A Light in the Attic", £51.77, 22 in stock, three stars (checked 2026-10-14).

## Leader

Stagehand 4.2.1, `extract` with a Zod schema (TypeScript).

## Pains

P19 model breadth and structured output, P04 false success, P42 cost visibility, P03 per-run AI cost; keeps K08 any model, including local ones.

## Failure points

1. The model's answer does not match the declared types (the price as `"£51.77"`, the rating as `"Three"`).
2. A field is absent from the page.
3. The model provider is unreachable, refuses, or rate-limits.
4. The model is not able to give structured output at all.
5. The page did not load, or loaded a different page.
6. Usage or cost is not reported.

## Setup

Written by #483.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Stagehand (TypeScript) | | | | |
| Goodfetch TypeScript | | | | |
| Goodfetch Python | | | | |
| Goodfetch Rust | | | | |

## After the sketch

A new Journey on the SDK surfaces (J-011 covers extraction without a model); Proving ground families: extraction and AI, served by the Fake model.
