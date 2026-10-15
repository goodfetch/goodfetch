# 12. Run a crawl inside a web service

## Task statement

> Write a small web service with one endpoint, `/books?category=<name>`, that crawls that category of https://books.toscrape.com/ (following its "next" pages) when called, and answers with its books as JSON (title and price). Use the language's usual minimal web framework (for example FastAPI, Node's `http` module, axum). Two calls may run at the same time in the same process without interfering, and calling it again later works the same. If the caller disconnects, the crawl stops and releases every connection and browser it used. The library's logs go to the service's own logger, and nothing is printed to standard output. Also call the same crawl from a notebook cell in Python, and from a plain script in TypeScript and Rust.

## Target

`books.toscrape.com`: category pages (checked 2026-10-14).

## Leader

Scrapy 2.19.0 (Python), run from a script with `AsyncCrawlerRunner` (no Twisted reactor) inside FastAPI.

## Pains

P05 embedding and re-entrancy, P06 teardown, P22 concurrency model friction, P23 cancellation, P44 logs under the host's control; keeps K04 plain async code with no hidden queue.

## Failure points

1. The second crawl in the same process fails (Scrapy's `ReactorNotRestartable` class of error).
2. Two crawls running at once share state, signal handlers or storage.
3. A disconnected caller leaves the crawl, or its browser, running.
4. The library prints to standard output.
5. The library installs process-wide signal handlers.
6. A blocking call fails inside the notebook's running event loop, or hangs.

## Setup

Counted apart from lines. Goodfetch's engine comes with each package and its pinned browser is fetched on first use (ADRs 0024, 0057); this crawl needs none, since books.toscrape.com is plain HTML.

| Program | Commands | Count |
|---|---|---|
| Scrapy (Python) | `pip install scrapy==2.19.0 fastapi uvicorn` (aiohttp, the download handler without a reactor, is a dependency of Scrapy) | 1 |
| Goodfetch TypeScript | `npm install @goodfetch/foxhound zod` (`@types/node` only for the type checker) | 1 |
| Goodfetch Python | `pip install goodfetch-foxhound fastapi uvicorn` | 1 |
| Goodfetch Rust | `cargo add goodfetch-foxhound axum serde --features serde/derive`; `cargo add tokio --features macros,rt-multi-thread,net,signal` | 2 |

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Scrapy (Python) | 29 | 65 | 1 | 6 / 0 / 0 |
| Goodfetch TypeScript | 41 | 83 | 1 | 6 / 0 / 0 |
| Goodfetch Python | 34 | 65 | 1 | 6 / 0 / 0 |
| Goodfetch Rust | 41 | 105 | 2 | 6 / 0 / 0 |

Programs: [`leaders/s12_crawl_in_a_service_scrapy.py`](leaders/s12_crawl_in_a_service_scrapy.py), [`../typescript/scenarios/12-crawl-in-a-service.ts`](../typescript/scenarios/12-crawl-in-a-service.ts), [`../python/scenarios/s12_crawl_in_a_service.py`](../python/scenarios/s12_crawl_in_a_service.py), [`../rust/examples/s12_crawl_in_a_service.rs`](../rust/examples/s12_crawl_in_a_service.rs).

Each program finds the category's link on the home page by its exact name, follows its next pages, answers 404 for a name the site does not have, and holds one connection (Goodfetch) or one runner (Scrapy) for the service's life, with a Run (a Crawler) per call. Starlette does not cancel an endpoint whose caller disconnects, so both Python services watch `request.receive()` for `http.disconnect`. How each Goodfetch Run ends:

- **TypeScript and Python:** `finish()`; a disconnected caller aborts its wait (`signal`, task cancellation), and leaving the scope then **closes** the Run, which stops it with a Checkpoint and closes its Sessions (ADRs 0078, 0102).
- **Rust:** `crawl.serve(&run)` returns the report once the Run ends (ADR 0102's Rust form of `finish()`), then `close()`; a disconnected caller makes axum **drop** the future, which drops `serve` (the program's own stop, ADR 0093) and then the unclosed Run, whose `Drop` stops it without blocking (ADR 0078).

The Python notebook cell is `notebook_cell()`, the cell's body written as a function: it awaits the same crawl with the async client on the notebook's running loop. The sync `Foxhound`, which works there too (ADR 0081), takes only a sync `Crawl`, so it would need the crawl declared a second time. TypeScript and Rust run the same crawl as a plain script when given a category name as their argument.

Leader: Scrapy 2.19.0's documented way to run spiders inside an asyncio application is `AsyncCrawlerRunner` with `TWISTED_REACTOR_ENABLED` set to `False` ("Running spiders inside existing applications"; experimental since 2.15, verified in the installed source), and its docs show the same for a Jupyter notebook. `CrawlerRunner` itself needs a running Twisted reactor and is unavailable without one.

**Concepts counted** (each name once per program; obligations last; FastAPI, Starlette and AnyIO, Node's `http` and axum are the web framework's, the schema libraries are not counted):

- Scrapy (Python), 29: `Spider`, `name`, `start_urls`, spider arguments passed to `__init__`, `parse`, `TextResponse`, `Response`, `xpath` (with a variable), `get`, `CloseSpider`, `follow`, `css`, `getall`, `follow_all`, `callback`, `signals`, `item_scraped`, `AsyncCrawlerRunner`, `TWISTED_REACTOR_ENABLED`, `Crawler`, `connect`, `signal`, `crawl`, `stats`, `get_value`, `"finish_reason"`, `create_crawler`, `stop_async`; must await.
- Goodfetch TypeScript, 41: `Crawl`, `Foxhound`, `seed`, `CallOptions`, `Logger`, `dataset`, `schema`, `label`, `payload`, `handle`, `page`, `request`, `enqueueLinks`, `skip`, `locator`, `getByRole`, `"link"`, `name`, `exact`, `added`, `push`, `all`, `getAttribute`, `textContent`, `startCrawl`, `seeds`, `finish`, `requests`, `bySkipReason`, `"handler"`, `records`, `value`, `connect`, `logger`, `signal`, the logger's `debug`, `info`, `warn`, `error`; must dispose (`await using`), must await.
- Goodfetch Python, 34: `AsyncCrawl`, `AsyncCrawlContext`, `AsyncFoxhound`, `seed`, `dataset`, `schema`, `label`, `payload`, `handle`, `request`, `page`, `locator`, `get_by_role`, `"link"`, `name`, `exact`, `enqueue_links`, `added`, `skip`, `all`, `get_attribute`, `push`, `text_content`, `start_crawl`, `seeds`, `finish`, `requests`, `by_skip_reason`, `"handler"`, `records`, `value`, `connect`; must close (`async with`), must await.
- Goodfetch Rust, 41: `AriaRole`, `Link`, `ConnectOptions`, `Crawl`, `CrawlContext`, `Error`, `Foxhound`, `Seed`, `SkipReason`, `Handler`, `new`, `dataset`, `label`, `handle`, `request`, `payload`, `page`, `locator`, `get_by_role`, `name`, `exact`, `enqueue_links`, `added`, `skip`, `all`, `get_attribute`, `text_content`, `push`, `start_crawl`, `serve`, `requests`, `by_skip_reason`, `records`, `next`, `value`, `as_run`, `close`, `connect`, `clone`; must await (builders), must close.

### Failure points, scored

A failure point that the program rules out by design counts as typed.

| Failure point | Scrapy (Python) | Goodfetch TypeScript | Goodfetch Python | Goodfetch Rust |
|---|---|---|---|---|
| 1. The second crawl in the process fails | typed (by design): with no reactor, each call's Crawler runs on the service's loop; with the default reactor, `CrawlerRunner` needs the program to run and keep one, and a stopped one is `ReactorNotRestartable` | typed (by design): one connection, a Run per call, nothing process-wide (ADR 0078) | typed (by design), as TypeScript | typed (by design), as TypeScript |
| 2. Two crawls at once share state, signal handlers or storage | typed (by design): a Crawler per call, each with its own signals and stats; no `JOBDIR` | typed (by design): each Run has its own Frontier, Datasets and Sessions (G2) | typed (by design), as TypeScript | typed (by design), as TypeScript |
| 3. A disconnected caller leaves the crawl or its browser running | typed: the program's watcher calls `crawler.stop_async()`, a graceful stop that lets in-flight requests finish; without it the crawl runs on | typed: the abort ends `finish()`'s wait, and closing the Run stops it and closes its Sessions | typed: the watcher cancels the task, and closing the Run stops it; without the watcher, as in Scrapy, Starlette lets the crawl run on | typed: axum drops the future; the dropped `serve` and Run stop it without blocking, with a `tracing` warning for the unclosed Run |
| 4. The library prints to standard output | typed (by design): a runner never calls `configure_logging()`, so Scrapy's `logging` records go to the service's handlers | typed (by design): only the `logger` given; with none, nothing | typed (by design): `logging` under `goodfetch`, with only a `NullHandler` | typed (by design): `tracing` events, for the service's subscriber |
| 5. The library installs process-wide signal handlers | typed (by design): `AsyncCrawlerRunner` installs none (`CrawlerProcess` does) | typed (by design): none (ADR 0078) | typed (by design), as TypeScript | typed (by design), as TypeScript |
| 6. A blocking call fails or hangs inside the notebook's running loop | typed (by design): the reactorless runner is awaited on the notebook's loop, as Scrapy's docs show | typed (by design): a plain script, no foreign loop | typed (by design): the async client awaited on the notebook's loop (ADR 0078); the sync client runs on its own thread (ADR 0081) | typed (by design), as TypeScript |

**Verdict.**

- Failure points: no Goodfetch version does better than Scrapy 2.19 run without a reactor, which removed the scenario's founding pain (`ReactorNotRestartable`). The difference is status: that mode is experimental, with Scrapy's own HTTP handler disabled in it, where Goodfetch's guest rules are its only mode (G4). Goodfetch's disconnect also closes the browser a crawl used; Scrapy's crawl has none to release.
- Lines: Goodfetch Python equals Scrapy's. Goodfetch TypeScript is longer because Node's `http` has no routing or query parsing and the program writes its own logger to standard error, since `console.info` writes to standard output.
- Concepts: Goodfetch Python names five more than Scrapy, for the declared Dataset and typed Label Payload (G3) and for reading "no such category" from the Run report (`bySkipReason`), where Scrapy's program reads its stats by a string key (G1, G6).
- Rust declares the crawl per call, since its handlers borrow that call's Dataset, and answers in axum by being dropped; it has no scored leader. Its handlers and the futures they return are `Send` (ADR 0112), so `serve`'s future, held by axum's handler future, is `Send` as axum requires; held as they were before, the handlers made `get(get_books)` fail to compile.

**Raised while writing it:** [Send bounds on a Rust crawl's handlers](https://github.com/goodfetch/goodfetch/issues/531), decided by ADR 0112.

## After the sketch

A new Journey on the SDK surfaces; Proving ground family: `catalog`.
