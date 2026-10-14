# 05. Crawl two page types into a Dataset

## Task statement

> Crawl https://books.toscrape.com/. From the home page, follow every category link in the side list except the top-level "Books" one. On each category page, follow its book links and its "next" page link. On each book page, record one item with the title, the price (a decimal number of pounds), the number in stock (an integer), the star rating (one to five), the page's address, and the name of the category it was reached from: that name is passed from the category page, not read again on the book page. Stay on the site. Store the items in a named dataset and export it to a JSON file when the crawl ends. Report how many pages succeeded, failed or were skipped, and why each failed or skipped page did.

## Target

`books.toscrape.com`: 50 categories in the side list, paginated category pages, 1,000 book pages (checked 2026-10-14).

## Leader

Crawlee 3.18.2 (TypeScript): `CheerioCrawler` with a router, labels and `userData`. Scrapy 2.19.0 (Python): a `Spider` with callbacks, `cb_kwargs` and feed exports. Crawlee 4.0.0-rc.1 changes are noted where they matter.

## Pains

P21 passing data between handlers, P49 the handler context, P48 failure, retry and skip, P12 politeness defaults, P04 honest counts; keeps K03 one API across fetch tiers.

## Failure points

1. A book page answers 404 or 500: retried, then failed with its cause.
2. The site slows down or answers 429: the crawl backs off instead of hammering it.
3. A field is missing on one book page.
4. The value passed from the category page is misspelled, missing or not serializable.
5. A link would lead off the site.
6. The same book is reached twice.
7. The export fails (disk full, path not writable).
8. The crawl ends with no item at all.

## Setup

Written by #483.

## Measures

| Program | Concepts | Lines | Setup | Failure points: typed / generic / silent |
|---|---|---|---|---|
| Crawlee (TypeScript) | | | | |
| Scrapy (Python) | | | | |
| Goodfetch TypeScript | | | | |
| Goodfetch Python | | | | |
| Goodfetch Rust | | | | |

## After the sketch

The SDK variants of J-002, or a new Journey if two page types make it a different goal; Proving ground families: `catalog` and the crawl families.
