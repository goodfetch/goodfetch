# 06. Crawl with HTTP and a browser only where needed

## Task statement

> Collect every quote (its text, author and tags) from three versions of the same site: https://quotes.toscrape.com/ (plain HTML), https://quotes.toscrape.com/js/ (rendered by JavaScript) and https://quotes.toscrape.com/scroll (infinite scrolling). Follow each version's pagination, and record each quote once per version. Fetch over plain HTTP wherever the content is in the HTML, and use a browser only for the pages that need one, without a second crawler or a rewritten handler for the browser pages. In the browser, do not download images or fonts. On the scrolling version, take the quotes from the JSON responses the page loads rather than from the screen. A page fetched over HTTP whose quotes are missing must not count as a success.

## Target

`quotes.toscrape.com`: `/` and its pages, `/js/` and its pages, and `/scroll`, which loads `/api/quotes?page=N` (checked 2026-10-14).

## Leader

Crawlee 3.18.2 (TypeScript): `AdaptivePlaywrightCrawler`. scrapy-playwright 0.0.48 with Scrapy 2.19.0 (Python).

## Pains

P11 switching fetch tier, P49 one handler context across tiers, P30 network interception and response bodies, P27 capabilities at runtime, P04 an empty page is not a success, P24 browser provisioning; keeps K03 one API across HTTP and browser fetching.

## Failure points

1. A page fetched over HTTP needs JavaScript, and its quotes are missing.
2. No browser is installed on the machine.
3. The scrolling page stops loading before the last page.
4. A quote is recorded twice across scroll loads.
5. Blocking images or fonts breaks the page.
6. The JSON response cannot be parsed.

## Setup

Counted apart from lines. Goodfetch's engine comes with each package and its pinned browser is fetched on first use (ADRs 0024, 0057).

| Program | Commands | Count |
|---|---|---|
| Crawlee (TypeScript) | `npm install crawlee@3.18.2 playwright@1.64.1 zod`; `npx playwright install chromium` | 2 |
| scrapy-playwright (Python) | `pip install scrapy==2.19.0 scrapy-playwright==0.0.48`; `playwright install chromium` | 2 |
| Goodfetch TypeScript | `npm install @goodfetch/foxhound zod` | 1 |
| Goodfetch Python | `pip install goodfetch-foxhound pydantic` | 1 |
| Goodfetch Rust | `cargo add goodfetch-foxhound serde serde_json --features serde/derive`; `cargo add tokio --features macros,rt-multi-thread` | 2 |

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Crawlee (TypeScript) | 35 | 100 | 2 | 1 / 5 / 0 |
| scrapy-playwright (Python) | 42 | 78 | 2 | 1 / 5 / 0 |
| Goodfetch TypeScript | 51 | 84 | 1 | 6 / 0 / 0 |
| Goodfetch Python | 52 | 98 | 1 | 6 / 0 / 0 |
| Goodfetch Rust | 71 | 127 | 2 | 6 / 0 / 0 |

Programs: [`leaders/06-mixed-fetch-crawl.crawlee.ts`](leaders/06-mixed-fetch-crawl.crawlee.ts), [`leaders/s06_mixed_fetch_crawl_scrapy_playwright.py`](leaders/s06_mixed_fetch_crawl_scrapy_playwright.py), [`../typescript/scenarios/06-mixed-fetch-crawl.ts`](../typescript/scenarios/06-mixed-fetch-crawl.ts), [`../python/scenarios/s06_mixed_fetch_crawl.py`](../python/scenarios/s06_mixed_fetch_crawl.py), [`../rust/examples/s06_mixed_fetch_crawl.rs`](../rust/examples/s06_mixed_fetch_crawl.rs).

Every program does the same work: one handler for the plain and the JavaScript versions, which waits for a quote so that an HTTP page without one goes to the browser; a browser-only handler for the scrolling version that reloads with a waiter armed for `/api/quotes?page=1` (page 1 loads during the engine's own navigation, before any handler can wait for it, in every library), then awaits each JSON page once by its number; images and fonts blocked in the browser. Notes on the leaders, verified in the installed sources:

- Crawlee 3.18.2 marks `AdaptivePlaywrightCrawler` `@experimental`. In the browser its context's `enqueueLinks` waits 5 s for its selector and throws when none appears, as on a version's last page, so the program enqueues only when a next link exists. A pre-navigation hook runs in HTTP-only mode too, where reading `page` throws and an uncaught throw sends the request to the browser, so the hook catches it.
- scrapy-playwright 0.0.48 decides the tier per request, by its `playwright` meta key; nothing is learned. The program fetches every plain and JavaScript page over HTTP and refetches in the browser one with no quote, so each `/js/` page is fetched twice.
- Goodfetch learns the tier per Label on a domain (`fetch.tier: auto`), so the plain and JavaScript versions are two Labels sharing one handler (ADR 0093); the scrolling Label forces `browser`. The before-navigation Hook is unscoped: it runs only where a browser navigates, so it also covers a sample comparison of the plain Label's pages in the browser, which a Hook scoped to the browser Labels would miss.

**Concepts counted** (each name once per program; obligations last; the schema libraries, Zod, Pydantic, serde and schemars, are not counted):

- Crawlee (TypeScript), 35: `AdaptivePlaywrightCrawler`, `createAdaptivePlaywrightRouter`, `playwrightUtils`, `AdaptivePlaywrightCrawlerContext`, `request`, `waitForSelector`, `parseWithCheerio`, `pushData`, `enqueueLinks`, Cheerio's `map`, `find`, `text`, `get`, `length`, `label`, `selector`, `addHandler`, `page`, Playwright's `waitForResponse`, `reload`, `json`, `mouse`, `wheel`, `requestHandler`, `preNavigationHooks`, `blockRequests`, `urlPatterns`, `run`, `url`, `requestsFinished`, `requestsFailed`, `getData`, `items`; must await, must catch the `page` read in a hook.
- scrapy-playwright (Python), 42: Playwright's `Page`, `Request`, `resource_type`, `Spider`, `name`, `custom_settings`, `DOWNLOAD_HANDLERS`, `ScrapyPlaywrightDownloadHandler`, `PLAYWRIGHT_ABORT_REQUEST`, `FEEDS`, `format`, `"jsonlines"`, `start`, `scrapy.Request`, `cb_kwargs`, `callback`, `meta`, `"playwright"`, `"playwright_include_page"`, `errback`, `parse`, `TextResponse`, `Response`, `css`, `follow`, `dont_filter`, `url`, `get`, `getall`, `follow_all`, `"playwright_page"`, `expect_response`, `reload`, `value`, `json`, `mouse`, `wheel`, `close`, `Failure`, `failure.request`; must close the page on success and in the errback, must await.
- Goodfetch TypeScript, 51: `Crawl`, `Foxhound`, `seed`, `Handler`, `Label`, `dataset`, `schema`, `label`, `settings`, `fetch`, `tier`, `"browser"`, `beforeNavigation`, `page`, `route`, `request`, `resourceType`, `"image"`, `"font"`, `abort`, `"blockedbyclient"`, `fallback`, `name`, `handle`, `push`, `enqueueLinks`, `locator`, `first`, `waitFor`, `state`, `"attached"`, `all`, `textContent`, `allTextContents`, `waitForResponse`, `reload`, `json`, `mouse`, `wheel`, `connect`, `startCrawl`, `seeds`, `finish`, `requests`, `succeeded`, `failed`, `byErrorCode`, `records`, `value`; must dispose (`await using`), must await.
- Goodfetch Python, 52: `AsyncCrawl`, `AsyncCrawlContext`, `AsyncFoxhound`, `AsyncPageStageContext`, `AsyncRoute`, `seed`, `dataset`, `schema`, `label`, `settings`, `fetch`, `tier`, `"browser"`, `request`, `resource_type`, `"image"`, `"font"`, `abort`, `"blockedbyclient"`, `fallback`, `before_navigation`, `name`, `page`, `route`, `handle`, `locator`, `first`, `wait_for`, `state`, `"attached"`, `all`, `push`, `text_content`, `all_text_contents`, `enqueue_links`, `expect_response`, `reload`, `value`, `json`, `mouse`, `wheel`, `connect`, `start_crawl`, `seeds`, `finish`, `requests`, `succeeded`, `failed`, `by_error_code`, `records`; must close (`async with`), must await.
- Goodfetch Rust, 71: `AbortReason`, `ConnectOptions`, `Crawl`, `CrawlContext`, `ElementState`, `Error`, `FetchTier`, `Foxhound`, `HookOn`, `LabelDeclaration`, `PageStageContext`, `ResourceType`, `Seed`, `Settings`, `UrlMatcher`, `settings::fetch`, `Regex`, `new`, `dataset`, `label`, `set`, `TIER`, `TierChoice`, `Forced`, `Browser`, `settings`, `every_label`, `named`, `before_navigation`, `page`, `route`, `for_each`, `next`, `request`, `resource_type`, `Image`, `Font`, `abort`, `reason`, `BlockedByClient`, `fallback`, `locator`, `first`, `wait_for`, `Attached`, `all`, `text_content`, `all_text_contents`, `name`, `push`, `enqueue_links`, `handle`, `wait_for_response`, `during`, `reload`, `json`, `skip`, `mouse`, `wheel`, `connect`, `start_crawl`, `serve`, `requests`, `succeeded`, `failed`, `by_error_code`, `records`, `value`, `close`; must await (builders), must close.

### Failure points, scored

A failure point that the program rules out by design counts as typed.

| Failure point | Crawlee (TypeScript) | scrapy-playwright (Python) | Goodfetch TypeScript and Python | Goodfetch Rust |
|---|---|---|---|---|
| 1. An HTTP page needs JavaScript, its quotes missing | generic: the fallback works, but it runs on `Error("Selector '.quote' not found.")`, logged as an exception on every HTTP attempt; with `querySelector` instead of a wait, an empty page passes | generic: the program's own check refetches in the browser; an empty browser page is the program's `ValueError` | typed: the wait escalates the request to the browser, learned per Label; under a forced tier, `BrowserRequiredError` naming `fetch.tier`; no quote in the browser either is `DeadlineExceededError`, then `failed` | typed, as TypeScript (`browser_required`, `deadline_exceeded`) |
| 2. No browser installed | generic: Playwright's launch `Error` ("Executable doesn't exist…"), read as text | generic: the same Playwright error inside the download handler | typed (by design): the pinned build is fetched on first use; offline, the error names the build and the install command (ADRs 0024, 0098) | typed (by design), as TypeScript |
| 3. Scrolling stops before the last page | generic: Playwright's `TimeoutError` after 30 s; the attempt's pushes are discarded and it is retried | generic: Playwright's `TimeoutError`, logged as a callback error; the items yielded before it are kept, so the version is partial with no retry | typed: `DeadlineExceededError` naming the awaited response and `timeout.wait`; the attempt's Records are discarded (ADR 0103) and it is retried, then `failed` | typed, as TypeScript |
| 4. A quote recorded twice across scroll loads | typed (by design): each JSON page awaited once by its number; a failed attempt's pushes are not committed | typed (by design): each JSON page awaited once; a failed callback is not retried | typed (by design): each JSON page awaited once; Records commit with their attempt only | typed (by design), as TypeScript |
| 5. Blocking images or fonts breaks the page | generic: the page's waits time out with `TimeoutError`; `blockRequests` matches URL patterns, so a font served without an extension still downloads | generic: the waits time out with `TimeoutError` | typed: the wait's `DeadlineExceededError` names what it waited for, and the Run trace lists each request the Hook's route aborted | typed, as TypeScript |
| 6. The JSON response cannot be parsed | generic: Playwright's `json()` throws a `SyntaxError`; a wrong shape is the program's `ZodError` | generic: `JSONDecodeError`; a wrong shape is a `KeyError` | typed: `ResponseNotJSONError`; a wrong shape is the schema's error, which ends the request `failed` with `handler_failed` naming it | typed: `response_not_json`; a wrong shape ends the request `skipped` with the deserializer's message (gap: a Rust handler cannot return its own error) |

**Verdict.**

- Lines: Goodfetch TypeScript is 16 lines shorter than Crawlee's: the page's own locators serve every tier, so no Cheerio parse, and the Hook needs no guard around a `page` that throws over HTTP. Goodfetch Python is 20 lines longer than scrapy-playwright's because it validates the API's JSON with Pydantic models and declares the Dataset's schema, where the leader indexes dictionaries unchecked (G3).
- Concepts: Goodfetch names more than both leaders. Its route spells the resource types, abort reason and fallback that Crawlee's `blockRequests` hides behind URL patterns and scrapy-playwright's `PLAYWRIGHT_ABORT_REQUEST` behind one setting, and the program reads its outcome from the Run report (`finish`, `requests`, `byErrorCode`), which no leader reports at all (G1, G6). Learning the tier per Label costs a second Label, `fetch.tier` and `"browser"` (S2: no per-request override).
- Failure points: every Goodfetch failure point is typed, where both leaders surface five as exceptions read as text; the escalation is learned and staged with its attempt, where Crawlee's runs on a thrown "not found" and scrapy-playwright's is the program's own refetch.
- Rust needs the most concepts and lines, from setting constants and builders; its Hook hands the route's stream to `ctx.for_each`, which `serve` reads for the rest of the attempt ([ADR 0111](https://github.com/goodfetch/goodfetch/blob/main/docs/adr/0111-a-rust-crawl-hands-what-it-subscribes-on-its-page-to-serve-which-reads-it-for-the-rest-of-the-attempt.md)), where the sketch first needed a task of the program's own; it has no scored leader.

**Raised while writing it:** [Reading the responses of a handler's own navigation](https://github.com/goodfetch/goodfetch/issues/529), [A route a Rust before-navigation Hook installs](https://github.com/goodfetch/goodfetch/issues/530) (ADR 0111), [A Rust crawl handler's own error](https://github.com/goodfetch/goodfetch/issues/528).

## After the sketch

The SDK variants of J-003 (a JavaScript-rendered crawl), or a new Journey for mixed tiers; Proving ground families: HTTP crawl and rendered crawl.
