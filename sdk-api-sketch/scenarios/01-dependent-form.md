# 01. Fill a dependent form and read the results

## Task statement

> Open https://quotes.toscrape.com/search.aspx. Choose the author "Albert Einstein": the page reloads and the tag list then offers that author's tags. Choose the tag "inspirational", press Search, and return every quote shown as a list of typed objects with the quote's text, its author and the tag. If no quote matches, return an empty list and say so; if the page does not behave as described, fail with an error that says which step and what was seen. Close everything the program opened before it ends.

## Target

`quotes.toscrape.com/search.aspx`: choosing an author posts the form (an ASP.NET ViewState postback) and reloads the page with that author's tags; Search posts it again to `/filter.aspx`. Albert Einstein and "inspirational" give one quote (checked 2026-10-14).

## Leader

Playwright 1.64.1 (TypeScript) and 1.64.0 (Python).

## Pains

P01 app state versus element readiness, P02 navigation races, P10 errors, P28 getting data out, P29 handles, P52 input actions, P06 scoped close; keeps K01 auto-waiting and K11 user-facing locators.

## Failure points

1. The page never reloads after the author is chosen, or reloads slowly.
2. The tag is not offered for that author.
3. The results are read before the second postback has finished loading.
4. No quote matches (an expected outcome, not an error).
5. The layout changed: a locator matches nothing, or several elements.
6. The site is unreachable or too slow.

## Setup

Written by #483.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Playwright (TypeScript) | | | | |
| Playwright (Python) | | | | |
| Goodfetch TypeScript | | | | |
| Goodfetch Python | | | | |
| Goodfetch Rust | | | | |

## After the sketch

A new Journey on the SDK surfaces; Proving ground family: rendered and interactive pages (scripted actions).
