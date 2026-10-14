# 09. Add an AI step to an existing Playwright program

## Task statement

> You have an existing Playwright program: it launches Chromium, opens https://books.toscrape.com/, clicks the "Travel" category, then clicks the first book and prints its title before closing the browser itself. Without rewriting it, add one step on the page it already drives, right after the "Travel" click: with a language model, extract the title and price of every book listed. The existing program then carries on with its page as before. The added step must not close, navigate or take over anything the program opened, and if it fails, the program must still be able to continue and close its browser.

## Target

`books.toscrape.com`: the Travel category page and a book page (checked 2026-10-14).

## Leader

Stagehand 3.7.3 on a Playwright page (TypeScript). Stagehand 4 refuses Playwright pages (ADR 0037), so version 3 is the latest that does this task.

## Pains

P20 interop with existing Playwright code, P41 driver guarantees, P06 never close what you did not start, P05 two libraries in one program.

## Failure points

1. The added step closes, or kills, the browser it did not launch.
2. The added step leaves the page navigated or in a different state.
3. The two libraries act on the page at the same time.
4. The browser was launched in a way the added step cannot attach to: the error must say how to launch it.
5. The model fails: the existing program continues.

## Setup

Written by #483.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Stagehand 3 with Playwright (TypeScript) | | | | |
| Goodfetch TypeScript | | | | |
| Goodfetch Python | | | | |
| Goodfetch Rust | | | | |

Lines count only what the added step needs, not the existing program.

## After the sketch

A new Journey on the SDK surfaces, with a Borrowed browser (ADR 0056); Proving ground family: rendered and interactive pages.
