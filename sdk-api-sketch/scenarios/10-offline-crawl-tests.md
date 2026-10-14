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

Written by #483.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Scrapy (Python) | | | | |
| Goodfetch TypeScript | | | | |
| Goodfetch Python | | | | |
| Goodfetch Rust | | | | |

## After the sketch

No Journey: it becomes the sample of the testing guide, built on the public testing kits of ADR 0069 (`goodfetch::testing`, `goodfetch/testing`, `goodfetch.testing`).
