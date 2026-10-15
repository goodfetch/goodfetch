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

Counted apart from lines. Goodfetch's engine comes with each package and its pinned browser is fetched on first use (ADRs 0024, 0057).

| Program | Commands | Count |
|---|---|---|
| Playwright (TypeScript) | `npm install playwright@1.64.1`; `npx playwright install chromium` | 2 |
| Playwright (Python) | `pip install playwright==1.64.0`; `playwright install chromium` | 2 |
| Goodfetch TypeScript | `npm install @goodfetch/collie` | 1 |
| Goodfetch Python | `pip install goodfetch-collie` | 1 |
| Goodfetch Rust | `cargo add goodfetch-collie`; `cargo add tokio --features macros,rt-multi-thread` | 2 |

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Playwright (TypeScript) | 24 | 37 | 2 | 1 / 5 / 0 |
| Playwright (Python) | 26 | 38 | 2 | 1 / 5 / 0 |
| Goodfetch TypeScript | 27 | 34 | 1 | 6 / 0 / 0 |
| Goodfetch Python | 26 | 44 | 1 | 6 / 0 / 0 |
| Goodfetch Rust | 39 | 57 | 2 | 6 / 0 / 0 |

Programs: [`leaders/01-dependent-form.playwright.ts`](leaders/01-dependent-form.playwright.ts), [`leaders/s01_dependent_form_playwright.py`](leaders/s01_dependent_form_playwright.py) (the sync API, which Playwright's Python docs show first), [`../typescript/scenarios/01-dependent-form.ts`](../typescript/scenarios/01-dependent-form.ts), [`../python/scenarios/s01_dependent_form.py`](../python/scenarios/s01_dependent_form.py) (the sync client, matching the leader), [`../rust/examples/s01_dependent_form.rs`](../rust/examples/s01_dependent_form.rs). Every program does the same steps, including the same check the site allows for a changed layout: its "Results" heading shows only above a match, so a heading over no quote fails instead of reading as "no match".

**Concepts counted** (each name once per program; obligations last):

- Playwright (TypeScript), 24: `chromium`, `launch`, `newPage`, `goto`, `getByLabel`, `selectOption`, `waitForResponse`, `getByRole`, `"button"`, `name`, `click`, `ok`, `status`, `waitForLoadState`, `locator`, `all`, `innerText`, `"heading"`, `waitFor`, `state`, `"hidden"`, `close`; must close the browser, must await.
- Playwright (Python), 26: `sync_playwright`, `chromium`, `launch`, `new_page`, `goto`, `get_by_label`, `select_option`, `expect_response`, `value`, `get_by_role`, `"button"`, `name`, `click`, `ok`, `status`, `wait_for_load_state`, `locator`, `all`, `inner_text`, `"heading"`, `wait_for`, `state`, `"hidden"`, `close`; must close the browser, must leave `sync_playwright()`.
- Goodfetch TypeScript, 27: `Collie`, `connect`, `start`, `settings`, `session`, `pageCheck`, `expectedPage`, `selector`, `newPage`, `goto`, `getByLabel`, `selectOption`, `getByRole`, `"button"`, `name`, `click`, `waitForLoadState`, `locator`, `all`, `textContent`, `"heading"`, `waitFor`, `state`, `"hidden"`, `finish`; must close (`await using`), must await.
- Goodfetch Python, 26: `Collie`, `connect`, `start`, `settings`, `session`, `page_check`, `expected_page`, `selector`, `new_page`, `goto`, `get_by_label`, `select_option`, `get_by_role`, `"button"`, `name`, `click`, `wait_for_load_state`, `locator`, `all`, `text_content`, `"heading"`, `wait_for`, `state`, `"hidden"`, `finish`; must close (`with`).
- Goodfetch Rust, 39: `settings::session`, `PAGE_CHECK`, `AriaRole`, `Button`, `Heading`, `Collie`, `ConnectOptions`, `scope`, `ElementState`, `Hidden`, `Error`, `LoadState`, `Load`, `PageCheck`, `new`, `expected_page`, `PageMatch`, `selector`, `SelectOption`, `Settings`, `set`, `start`, `settings`, `new_page`, `goto`, `get_by_label`, `select_option`, `get_by_role`, `name`, `click`, `wait_for_load_state`, `locator`, `all`, `text_content`, `wait_for`, `finish`; must await (builders), must close (`scope`), must use the Run report (`#[must_use]`). `Default::default` and `From::from` are the standard library's.

**Failure points, scored**

| Failure point | Playwright (TypeScript) | Playwright (Python) | Goodfetch (all three) |
|---|---|---|---|
| 1. No reload, or a slow one | generic: `TimeoutError` of the tag's `selectOption`; "did not find some options" only in its call log | generic: `TimeoutError`, as TypeScript | typed: `DeadlineExceededError` naming `locator.selectOption` on `getByLabel('Tag')`, the options observed and `timeout.action` with its layer |
| 2. Tag not offered | generic: the same `TimeoutError`, told apart from 1 only by reading the call log | generic: as TypeScript | typed: the same `DeadlineExceededError`, its `observed` listing the author's tags |
| 3. Results read before the postback loaded | generic: prevented by the click's navigation wait and `waitForLoadState()`; a load that never comes is a `TimeoutError` | generic: as TypeScript | typed: prevented by the click's wait for commit and `waitForLoadState()`; a load that never comes is `DeadlineExceededError` naming `timeout.wait` |
| 4. No quote matches | typed: an empty list, said so by the program | typed: as TypeScript | typed: an empty list, said so by the program |
| 5. Layout changed | generic: a field locator matching nothing is a `TimeoutError`, several a strict-mode `Error` listing them as text; a heading over no quote is the heading check's `TimeoutError` | generic: as TypeScript | typed: `TargetNotUniqueError` with every match rendered, `DeadlineExceededError` for nothing; a page without the form fails the Page check (`PageCheckFailedError`) |
| 6. Site unreachable or slow | generic: `goto` throws an `Error` carrying `net::ERR_…` as text, or a `TimeoutError`; a 500 from `filter.aspx` is the program's own `Error` | generic: as TypeScript (`playwright.sync_api.Error`, `RuntimeError`) | typed: `NavigationFailedError` with `netError` (retryable), `DeadlineExceededError` naming `timeout.navigation`; an error page fails the Page check |

**Verdict.** Goodfetch TypeScript is three lines shorter and three concepts larger than Playwright's: the Page check (`session.pageCheck.expectedPage`) replaces the leader's response-status check and also covers the first two navigations, and `finish()` ends the Run explicitly (ADR 0102, G1). Goodfetch Python is six lines longer for the same reason, ruff splitting the nested settings literal over nine lines, with equal concepts. Every Goodfetch failure is typed where Playwright's is a `TimeoutError` or an `Error` read as text, because each error carries its operation, target, `observed` and `limit` (G6). Rust needs the most concepts for one program, from builders, setting constants and the `#[must_use]` report, the price of S1 in Rust (C-BUILDER); it has no scored leader.

## After the sketch

A new Journey on the SDK surfaces; Proving ground family: rendered and interactive pages (scripted actions).
