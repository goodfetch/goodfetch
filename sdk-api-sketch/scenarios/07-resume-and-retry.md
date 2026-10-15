# 07. Resume a killed crawl and retry its failures

## Task statement

> Run the crawl of scenario 05 under a name you choose. Kill the process part-way through. Start the program again with the same name: the crawl continues where it stopped, and the dataset ends with every book exactly once. Then retry only the requests that failed in that crawl. Run it a third time with nothing left to do: it must say that nothing was done and why, instead of finishing silently. While the crawl runs, the program can print what is in flight and what is waiting, and why.

## Target

`books.toscrape.com`, as in scenario 05.

## Leader

Scrapy 2.19.0 (Python): `JOBDIR`. Crawlee 3.18.2 (TypeScript): persisted storage with `purgeOnStart: false`. Crawlee 4.0.0-rc.1, whose repeated `run()` keeps its queue and which explains a run that processes nothing, is noted.

## Pains

P17 run state surprises, P18 duplicates on retry, P09 live status, P08 deadlines (shutdown); keeps K05 seeing what happened.

## Failure points

1. Items written just before the kill are written again after the resume.
2. The second start silently does nothing, because every request is already marked as seen.
3. Retrying failures also re-runs requests that succeeded.
4. The saved state is missing or damaged.
5. The program changed between the kill and the resume.
6. The crawl hangs while shutting down.

## Setup

- **Scrapy (Python):** `pip install scrapy==2.19.0`, then `python s07_resume_and_retry_scrapy.py`. `JOBDIR` creates its folder itself.
- **Crawlee (TypeScript):** `npm install crawlee@3.18.2 zod@4.6.7 tsx`, then `npx tsx 07-resume-and-retry.crawlee.ts`.
- **Goodfetch TypeScript:** `npm install @goodfetch/foxhound zod tsx`, then `npx tsx 07-resume-and-retry.ts`.
- **Goodfetch Python:** `pip install goodfetch-foxhound pydantic anyio` (AnyIO, which the SDK runs on, receives the signals), then `python -m scenarios.s07_resume_and_retry`.
- **Goodfetch Rust:** `cargo add goodfetch-foxhound serde tokio -F serde/derive,tokio/macros,tokio/rt-multi-thread,tokio/signal`, then `cargo run --example s07_resume_and_retry`.

Each program is started three times by hand: run and kill it, run it again, run it once more. Every program reuses scenario 05's crawl by importing it, the leaders included: Scrapy subclasses its spider, and Crawlee imports its router. Crawlee 4.0.0-rc.1, checked in its published `@crawlee/basic`, still purges the default storages on start (`purgeOnStart` defaults to `true`). A repeated `run()` in one process keeps the queue, and a run that processes nothing logs why: every request was already handled.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Scrapy (Python) | 46 | 77 | 1 command | 0 / 3 / 3 |
| Crawlee (TypeScript) | 35 | 43 | 1 command | 0 / 4 / 2 |
| Goodfetch TypeScript | 38 | 42 | 1 command | 6 / 0 / 0 |
| Goodfetch Python | 39 | 63 | 1 command | 6 / 0 / 0 |
| Goodfetch Rust | 44 | 106 | 1 command | 6 / 0 / 0 |

Lines were counted with the brief's `measure_lines.py`. The Rust count is of the file as rustfmt leaves it, since the script's copy cannot resolve the `#[path]` module. Only each program's own lines and names are counted, not those of scenario 05's crawl, which it imports. Every program does the same things: resume, retry the failures once, say why a third run did nothing, print progress, and export each book once. Each Goodfetch program stops its Run on Ctrl+C (SIGINT) and on a plain `kill` (SIGTERM), in its language's idiom: `process.once` in TypeScript, AnyIO's `open_signal_receiver` in a task group in Python, which like asyncio's signal handlers works on Unix only, and Tokio's `ctrl_c()` beside `signal(SignalKind::terminate())` in Rust, whose SIGTERM part is behind `#[cfg(unix)]`. AnyIO's and Tokio's names are the program's own executor's and are not counted, as Node's `process` is not.

**Concepts counted.**

- **Scrapy:** `signals`, `Crawler`, `CrawlerProcess`, `DontCloseSpider`, `IgnoreRequest`, `HttpError`, `create_looping_call`, `Failure`, `name`, `custom_settings`, `JOBDIR`, `FEEDS`, `format`, `"jsonlines"`, `encoding`, `state`, `from_crawler`, `connect`, `spider_idle`, `spider_opened`, the looping call's `start` and `stop`, `failure.check`, `failure.request`, `url`, `callback`, `cb_kwargs`, `Request`, `errback`, `dont_filter`, `engine.crawl`, `logger`, `get_value`, `scheduler/enqueued`, `scheduler/dequeued`, `downloader.active`, `slots`, `queue`, `delay`, `closed`, `item_scraped_count`, `dupefilter/filtered`, `process.crawl`, `process.start`, plus two obligations: a single Ctrl+C (a second one, or a kill, loses what was in flight) and `start()` blocking.
- **Crawlee:** `CheerioCrawler`, `Configuration`, `Dataset`, `KeyValueStore`, `RequestQueue`, `purgeOnStart`, `open`, `config`, `getValue`, `setValue`, `requestQueue`, `requestHandler`, `failedRequestHandler`, `url`, `label`, `userData`, `statusMessageLoggingInterval`, `statusMessageCallback`, `crawler`, `state`, `getInfo`, `pendingRequestCount`, `autoscaledPool`, `currentConcurrency`, `desiredConcurrency`, `requestsFinished`, `requestsFailed`, `run`, `requestsTotal`, `uniqueKey`, `purgeRequestQueue`, `getData`, `items`, plus two obligations: `run()` resolving at the end, and Ctrl+C pausing (a second one forces the exit).
- **Goodfetch TypeScript:** `Foxhound`, `CrawlRun`, `connect`, `startCrawl`, `runId`, `seeds`, `id`, `startedAs`, `"ended"`, `stop`, `finish`, `progress`, `requests`, `pending`, `succeeded`, `failed`, `skipped`, `inFlight`, `request`, `url`, `phase`, `tier`, `attempt`, `waiting`, `reason`, `setting`, `value`, `lifecycle`, `"finished"`, `retryFailed`, `nothingDone`, `export`, `format`, `"json"`, `saveAs`, `records`, plus two obligations: `await using` disposal and `finish()`.
- **Goodfetch Python:** `AsyncCrawlRun`, `AsyncFoxhound`, `CrawlRunReport`, `connect`, `start_crawl`, `seeds`, `run_id`, `id`, `started_as`, `"ended"`, `stop`, `finish`, `progress`, `requests`, `pending`, `succeeded`, `failed`, `skipped`, `in_flight`, `request`, `url`, `phase`, `tier`, `attempt`, `waiting`, `reason`, `setting`, `value`, `lifecycle`, `"finished"`, `retry_failed`, `nothing_done`, `export`, `format`, `"json"`, `save_as`, `records`, plus two obligations: `async with` and `finish()`.
- **Goodfetch Rust:** `ConnectOptions`, `CrawlRun`, `CrawlRunReport`, `Error`, `ExportFormat`, `Foxhound`, `Lifecycle`, `StartedAs`, `scope`, `start_crawl`, `run_id`, `serve`, `stop`, `progress`, `next`, `requests`, `pending`, `succeeded`, `failed`, `skipped`, `in_flight`, `request`, `url`, `phase`, `tier`, `attempt`, `waiting`, `reason`, `setting`, `value`, `as_json`, `started_as`, `Ended`, `Finished`, `retry_failed`, `run`, `nothing_done`, `lifecycle`, `export`, `Json`, `save_as`, `records`, plus two obligations: `scope` closing the connection, and builders that send nothing until awaited. Dropping `serve` on Ctrl+C or SIGTERM is the program's own stop (ADR 0093).

### Failure points, scored

| Failure point | Scrapy | Crawlee | Goodfetch (TypeScript, Python, Rust) |
|---|---|---|---|
| 1. Items written again after the resume | silent: requests popped from the disk queue are lost at a kill, so books go missing with no word, and the appended feed may end on a cut line | silent: `pushData` writes at once, so a request re-run after a stop writes its book twice; only the program's deduplication at export hides it | typed: Records commit with their attempt; an attempt in flight at the kill is discarded, counted as `handler_lost` and run again |
| 2. Second start does nothing | generic: the resume works, but a third run's "nothing done" is the program's guess from `item_scraped_count` and `dupefilter/filtered` | generic: the resume works, but a third run's "nothing done" is the program's own check of `requestsTotal` | typed: `startedAs()` says `resumed` or `ended`, and `nothingDone` says `already_ended` |
| 3. Retrying re-runs successes | generic: the program keeps its failures in `spider.state` and re-sends them with `dont_filter`; nothing in Scrapy retries a crawl's failures | generic: the program keeps its failures in a key-value store and re-adds them under new `uniqueKey`s, since a failed request stays handled | typed: `retryFailed()` re-admits only `failed` requests in the same Run and answers how many |
| 4. Saved state missing or damaged | silent: a missing `JOBDIR` starts a new crawl that appends to the feed; a damaged one raises a pickle error | silent: missing storage starts a new crawl from the start URL | typed: `run_state_unavailable` names the state and its path, and never starts afresh under the ID |
| 5. Program changed | silent: changed start URLs or settings are not noticed; a renamed callback fails each request as it is unpickled | generic: a label with no handler fails each request with an `Error` message as it comes up | typed: `run_conflict` lists each changed input, and `label_unknown`, `payload_schema_changed`, `dataset_unknown` and `dataset_key_invalid` come before any work |
| 6. Hangs while shutting down | generic: one Ctrl+C waits for the downloads in flight up to `DOWNLOAD_TIMEOUT` (180 s), logged as text | generic: one Ctrl+C waits for the running handlers up to their timeout, logged as text | typed: Ctrl+C and a plain `kill` both stop the Run, and `stop()` (or `close()`) is bounded by `timeout.close` and raises `DeadlineExceededError` naming it |

**Verdict.**

- **Python:** Goodfetch's program is shorter than Scrapy's (63 lines against 77) and names fewer concepts (39 against 46). Resuming, retrying failures, explaining a run that did nothing, and saying why requests wait are calls on the Run. Scrapy needs a signal handler, the spider's pickled state, `dont_filter` and the downloader's internals for them (S2, G6).
- **TypeScript:** Goodfetch is level with Crawlee on length (42 lines against 43) and names three more concepts (38 against 35). Those are the progress fields that say why requests wait, which Crawlee cannot say (G6).
- **Failure points:** Goodfetch is better on every one. Every failure point is typed by default, and the Run's ID replaces the folder each leader infers state from (G2). Each leader has at least two silent failure points.

## After the sketch

The SDK variants of J-005 (resume after a kill, with no Record delivered twice); Proving ground families: `catalog`, and incident and recovery.
