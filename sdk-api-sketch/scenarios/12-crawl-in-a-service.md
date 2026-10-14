# 12. Run a crawl inside a web service

## Task statement

> Write a small web service with one endpoint, `/books?category=<name>`, that crawls that category of https://books.toscrape.com/ (following its "next" pages) when called, and answers with its books as JSON (title and price). Use the language's usual minimal web framework (for example FastAPI, Node's `http` module, axum). Two calls may run at the same time in the same process without interfering, and calling it again later works the same. If the caller disconnects, the crawl stops and releases every connection and browser it used. The library's logs go to the service's own logger, and nothing is printed to standard output. Also call the same crawl from a notebook cell in Python, and from a plain script in TypeScript and Rust.

## Target

`books.toscrape.com`: category pages (checked 2026-10-14).

## Leader

Scrapy 2.19.0 (Python), run from a script with `CrawlerRunner` inside FastAPI.

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

Written by #483.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Scrapy (Python) | | | | |
| Goodfetch TypeScript | | | | |
| Goodfetch Python | | | | |
| Goodfetch Rust | | | | |

## After the sketch

A new Journey on the SDK surfaces; Proving ground family: `catalog`.
