"""Crawl books.toscrape.com: home page -> category pages (with pagination) -> book pages.

Each book page yields one item. The category name travels as the Payload from the category page to the book page.
"""

from __future__ import annotations

import re
from pathlib import Path

from pydantic import BaseModel, Field

from goodfetch_foxhound import (
    Crawl,
    CrawlContext,
    CrawlRequestBlocked,
    CrawlRequestFailed,
    CrawlRequestOutcome,
    CrawlRequestOutcomeUnknown,
    CrawlRequestSkipped,
    CrawlRequestSucceeded,
    CrawlRunSettings,
    Foxhound,
    LinkInfo,
    seed,
)

START_URL = "https://books.toscrape.com/"
DATASET_NAME = "books"
OUTPUT_PATH = Path("books.json")

RATINGS = {"One": 1, "Two": 2, "Three": 3, "Four": 4, "Five": 5}


class CategoryPayload(BaseModel):
    """What a category page passes on: the name of the category the request was reached from."""

    category: str


class Book(BaseModel):
    title: str
    price: float
    in_stock: int
    rating: int = Field(ge=1, le=5)
    url: str
    category: str


crawl = Crawl()

home_label = crawl.label("home")
category_label = crawl.label("category", payload=CategoryPayload)
book_label = crawl.label("book", payload=CategoryPayload)
books = crawl.dataset(DATASET_NAME, schema=Book, key=["url"])


def _category_payload(link: LinkInfo) -> CategoryPayload | None:
    name = " ".join(link.text.split())
    if not name or name == "Books":
        return None
    return CategoryPayload(category=name)


@crawl.handle(home_label)
def handle_home(ctx: CrawlContext[None]) -> None:
    # The nested list holds the categories; the top-level "Books" link is outside it.
    links = ctx.page.locator("div.side_categories ul ul a")
    ctx.enqueue_links(links, category_label, _category_payload)


@crawl.handle(category_label)
def handle_category(ctx: CrawlContext[CategoryPayload]) -> None:
    payload = ctx.request.payload
    ctx.enqueue_links(ctx.page.locator("article.product_pod h3 a"), book_label, payload)
    ctx.enqueue_links(ctx.page.locator("li.next a"), ctx.request.label, payload)


@crawl.handle(book_label)
def handle_book(ctx: CrawlContext[CategoryPayload]) -> None:
    page = ctx.page
    title = page.locator("div.product_main h1").text_content().strip()

    price_text = page.locator("div.product_main p.price_color").text_content()
    price_match = re.search(r"\d+(?:\.\d+)?", price_text)
    if price_match is None:
        raise ValueError(f"no price in {price_text!r}")

    stock_text = page.locator("div.product_main p.availability").text_content()
    stock_match = re.search(r"\d+", stock_text)
    if stock_match is None:
        raise ValueError(f"no stock count in {stock_text!r}")

    rating_class = page.locator("div.product_main p.star-rating").get_attribute("class") or ""
    rating = next((value for word, value in RATINGS.items() if word in rating_class.split()), None)
    if rating is None:
        raise ValueError(f"no star rating in {rating_class!r}")

    ctx.push(
        books,
        Book(
            title=title,
            price=float(price_match.group(0)),
            in_stock=int(stock_match.group(0)),
            rating=rating,
            url=page.url,
            category=ctx.request.payload.category,
        ),
    )


def _describe(outcome: CrawlRequestOutcome) -> str | None:
    """A line about a request that failed, was skipped or was blocked; None for a success."""
    ref = outcome.request
    where = f"{ref.url} [{ref.label}]"
    if isinstance(outcome, CrawlRequestSucceeded):
        return None
    if isinstance(outcome, CrawlRequestFailed):
        return f"failed  {where}: {outcome.error.code}: {outcome.error.message}"
    if isinstance(outcome, CrawlRequestSkipped):
        detail = f": {outcome.message}" if outcome.message else ""
        return f"skipped {where}: {outcome.reason}{detail}"
    if isinstance(outcome, CrawlRequestBlocked):
        return f"blocked {where}: {outcome.block_cause}"
    if isinstance(outcome, CrawlRequestOutcomeUnknown):
        return f"unknown {where}: {outcome.raw}"
    return None


def main() -> None:
    settings: CrawlRunSettings = {"crawl": {"scope": {"strategy": "same_hostname"}}}
    with Foxhound.connect() as foxhound:
        with foxhound.start_crawl(crawl, seeds=[seed(START_URL, home_label)], settings=settings) as run:
            report = run.finish()

            # An exported file of the Dataset, written when the crawl ends.
            run.export(books, format="json").save_as(OUTPUT_PATH)

            # The last outcome of each request, keyed by request ID.
            final: dict[str, CrawlRequestOutcome] = {}
            with run.outcomes() as outcomes:
                for outcome in outcomes:
                    final[outcome.request.id] = outcome

    totals = report.requests
    print(f"succeeded: {totals.succeeded}")
    print(f"failed:    {totals.failed}")
    print(f"skipped:   {totals.skipped}")
    print(f"blocked:   {totals.blocked}")
    print(f"items stored in '{DATASET_NAME}': {report.records[DATASET_NAME].committed} (exported to {OUTPUT_PATH})")
    if totals.by_error_code:
        print(f"failures by error code: {dict(totals.by_error_code)}")
    if totals.by_skip_reason:
        print(f"skips by reason: {dict(totals.by_skip_reason)}")
    for outcome in final.values():
        line = _describe(outcome)
        if line is not None:
            print(line)


if __name__ == "__main__":
    main()
