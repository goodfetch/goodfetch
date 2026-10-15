# 10. Test a crawl offline

## Task statement

> Write tests for the crawl of scenario 05 that run with no network and no model, with the language's usual test runner. First, a unit test of the book-page handler fed a saved copy of one book page, checking the item it produces, including the category passed to it. Second, a test that runs the whole crawl from the responses recorded during one earlier live run, and checks that the dataset holds the expected number of books. Third, for the AI extraction of scenario 08, a test that runs from what was learned on the earlier run, or from a scripted model reply, and fails if a real model is called. Any test that would reach the network must fail, saying which address it tried.

## Target

Saved and recorded responses of `books.toscrape.com`; no live site.

## Leader

Scrapy 2.19.0 (Python): contracts, `HtmlResponse` built from a saved page, and the HTTP cache.

## Pains

P31 test-runner coupling, P03 replay without the model, P55 offline reruns from kept responses; keeps K07 testable callbacks and offline replay.

## Failure points

1. A test reaches the network without anyone noticing.
2. The recording is older than the site's current pages.
3. Testing one handler needs the whole engine and a browser.
4. A test calls a real model.
5. The tests only work with one test runner.

## Setup

Every version keeps the same saved page, fetched once: `curl -o tests/fixtures/a-light-in-the-attic.html https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html`. None needs a browser: the pages are static, and Goodfetch's `runHandler` runs on the `http` tier.

- **Scrapy (Python):** `pip install scrapy==2.19.0 pytest`; fill the HTTP cache with one live run, `scrapy runspider s05_two_page_crawl_scrapy.py -s HTTPCACHE_ENABLED=True -s HTTPCACHE_DIR="$PWD/tests/httpcache"`, and commit `tests/httpcache`; then `pytest s10_offline_crawl_tests_scrapy.py`. 3 commands.
- **Goodfetch TypeScript:** `npm install @goodfetch/foxhound zod tsx`; record one live run of scenario 05 by adding `recording: { mode: "record", path: "tests/books.warc.gz" }` to its `startCrawl`, and keep `books.plan.json` from one live run of scenario 08 (its model key exported for that run only); commit both; then `npx tsx --test scenarios/10-offline-crawl-tests.ts` (Node's own test runner, through tsx as scenario 05 runs). 4 steps.
- **Goodfetch Python:** `pip install goodfetch-foxhound pydantic pytest anyio` (AnyIO's pytest plugin runs the `async def` tests); the same recording (`recording=Recording(mode="record", path=...)`) and Plan file; then `pytest scenarios/s10_offline_crawl_tests.py`. 4 steps.
- **Goodfetch Rust:** `cargo add goodfetch-foxhound serde serde_json tokio -F serde/derive,tokio/macros,tokio/rt-multi-thread`, with a `test-util = ["goodfetch-foxhound/test-util"]` feature as the sketch's `Cargo.toml` declares it; the same recording (`.recording(Recording::record(..))`) and Plan file; then `cargo test --example s10_offline_crawl_tests --features test-util`. 4 steps.

pytest collects a file named on its command line whatever its name, so neither Python file needs a `test_` prefix. Scrapy's contracts (`@url`, `@cb_kwargs`, `@returns`, `@scrapes`, run by `scrapy check`) fetch the live URL each names, so the Scrapy program does not use them; its other two testing tools, an `HtmlResponse` built from a saved page and the HTTP cache with `HTTPCACHE_IGNORE_MISSING`, are its best documented offline configuration. Scrapy has no AI extraction, so the third test has no Scrapy counterpart: the step to keep a Plan file is Goodfetch's fourth.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Scrapy (Python) | 16 | 34 | 3 commands | 0 / 3 / 1 (one not applicable) |
| Goodfetch TypeScript | 31 | 48 | 4 steps | 5 / 0 / 0 |
| Goodfetch Python | 32 | 62 | 4 steps | 5 / 0 / 0 |
| Goodfetch Rust | 39 | 89 | 4 steps | 5 / 0 / 0 |

Lines count the test files only, not scenarios 05 and 08 they import, after oxfmt, ruff format and rustfmt (the Rust file checked with `rustfmt --check` beside the modules its `#[path]` names). The Scrapy program has two tests where Goodfetch's have three, since Scrapy has no AI step to test. The test runner's names (`test`, `assert`, `pytest.fixture`, `pytest.mark.anyio`, `#[tokio::test]`) are not counted, nor are the names scenarios 05 and 08 define (`crawl`, `books`, `book`, `seeds`, `BooksSpider`, `parse_book`).

**Concepts counted.**

- **Scrapy:** `AsyncCrawlerProcess`, its `settings`, `create_crawler`, `crawl`, `start`, `stats`, `get_value`, `HtmlResponse`, `body`, `encoding`, `HTTPCACHE_ENABLED`, `HTTPCACHE_DIR`, `HTTPCACHE_IGNORE_MISSING`, the stat keys `httpcache/miss` and `item_scraped_count`, plus one obligation: the reactor starts once per process, so a session runs one crawl.
- **Goodfetch TypeScript:** `seed`, `TestEngine`, `start`, `runHandler`, `url`, `payload`, `response`, `path`, `ended`, `kind`, `"returned"`, `records`, `connection`, `startCrawl`, `seeds`, `recording`, `mode`, `"replay"`, `finish`, `misses`, `committed`, `plan`, `settings`, `plan.frozen`, `requests`, `failed`, `usage`, `model`, `calls`, plus two obligations: `await using` disposal, and awaiting each call.
- **Goodfetch Python:** `seed`, `Recording`, `mode`, `"replay"`, `path`, `AsyncTestEngine`, `start`, `Returned`, `SavedResponse`, `run_handler`, `url`, `payload`, `response`, `ended`, `records`, `connection`, `start_crawl`, `seeds`, `recording`, `finish`, `misses`, `committed`, `plan`, `settings`, `plan.frozen`, `requests`, `failed`, `usage`, `model`, `calls`, plus two obligations: `async with`, and awaiting each call.
- **Goodfetch Rust:** `settings::plan`, `FROZEN`, `AttemptEnd`, `Returned`, `HandlerInput`, `new`, `response`, `SavedResponse`, `path`, `TestEngine`, `start`, `TestOptions`, `CrawlRunReport`, `Error`, `Recording`, `replay`, `Seed`, `Settings`, `set`, `run_handler`, `ended`, `records`, `connection`, `start_crawl`, `recording`, `plan`, `settings`, `serve`, `run`, `misses`, `url`, `committed`, `requests`, `failed`, `usage`, `model`, `calls`, plus two obligations: `close()` on the engine, and builders that send nothing until awaited. Rust ends each crawl with `crawl.serve(&run)`, which returns the Run report once the Run ends (ADR 0102's Rust rule), as scenario 05 does.

### Failure points, scored

| Failure point | Scrapy | Goodfetch (TypeScript, Python, Rust) |
|---|---|---|
| 1. A test reaches the network unnoticed | generic: with `HTTPCACHE_IGNORE_MISSING` a miss is dropped with `IgnoreRequest`, its URL only in the log's text; the test sees a count (`httpcache/miss`). Without that setting, the miss is fetched live, silently | typed: the test engine replays an empty recording when none is named, so a request it lacks is never sent and fails with `response_not_recorded` (`RefusedError`: method, URL, nearest recorded URLs; `fix`: record again or answer it with a route); both replaying tests assert the report's `misses` are empty, so a failure prints each missed URL |
| 2. The recording is older than the site | silent: the cache never expires by default and reports no age; with `HTTPCACHE_EXPIRATION_SECS` set, an expired entry is fetched live even under `HTTPCACHE_IGNORE_MISSING` | typed: the Run report's `recording` gives `recordedFrom`, `recordedTo` and the engine that wrote it, the Run is marked replayed, and Provenance marks each Record; the age is reported, never a failure, and a refresh is a reviewed `record` Run (ADR 0105) |
| 3. One handler needs the whole engine and a browser | generic: the callback runs alone on an `HtmlResponse`, but its outcome is only what it yields: this spider's skip is an empty list, the reason left in the spider's own list, and an error a raw exception | typed: `runHandler` runs the handler as the crawl wired it, with no Frontier, no Hooks and no browser on the `http` tier; `ended` is a typed `AttemptEnd` (`skipped` with reason and message, `escalated` naming the call that needs a browser, `failed` with its code) |
| 4. A test calls a real model | not applicable: Scrapy has no AI extraction | typed: the test engine has no Model provider unless the test registers one, so a model call fails with `model_not_configured`; under `plan.frozen` a step the Plan lacks fails with `plan_step_missing` instead; the test asserts `usage.model.calls` is 0 |
| 5. The tests work with one runner only | generic: plain functions with pytest's `assert`, but the Twisted reactor starts once per process, so a second crawl in the same session fails with `ReactorNotRestartable` | typed: no runner plugin; the kit's values close with `await using`, `async with` or `close()`, and failures are `GoodfetchError`s, so `node:test`, Vitest, pytest with AnyIO or `unittest`, and `cargo test` run them alike |

**Verdict.**

- **Concepts and lines:** Goodfetch's programs are longer and name more (48 to 89 lines and 31 to 39 concepts, against Scrapy's 34 and 16). They test more: a third test for the AI extraction, which Scrapy cannot write, the attempt's typed ending, and the recording's misses by address. Scrapy's callback test is shorter because it calls a plain method and reads what it yields, where `runHandler` runs the handler as the crawl wired it and reports how its attempt ended (G1, G6). Goodfetch's Python is 14 lines longer than its TypeScript: the three-line engine fixture and AnyIO's mark, ruff splitting the long test signatures, and the `recording` check that narrows an optional; Rust adds the engine's explicit `close()` and its included modules (explicit close: G4).
- **Setup:** one step more for Goodfetch, the Plan file kept from scenario 08's live run, which is the AI test Scrapy lacks.
- **Failure points:** Goodfetch is better on all five, none silent: a miss is refused and named rather than dropped or fetched (G1, G5), the recording's age is reported (G6), and no model can be called unless the test registers one, with no process-wide switch (G2).

## After the sketch

No Journey: it becomes the sample of the testing guide, built on the public testing kits of ADR 0069 (`goodfetch::testing`, `goodfetch/testing`, `goodfetch.testing`).
