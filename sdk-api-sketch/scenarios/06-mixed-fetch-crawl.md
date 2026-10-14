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

Written by #483.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Crawlee (TypeScript) | | | | |
| scrapy-playwright (Python) | | | | |
| Goodfetch TypeScript | | | | |
| Goodfetch Python | | | | |
| Goodfetch Rust | | | | |

## After the sketch

The SDK variants of J-003 (a JavaScript-rendered crawl), or a new Journey for mixed tiers; Proving ground families: HTTP crawl and rendered crawl.
