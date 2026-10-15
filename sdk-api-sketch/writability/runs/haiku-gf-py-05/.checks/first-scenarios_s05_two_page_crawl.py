"""Crawls https://books.toscrape.com/: every category in the side list, its book pages and its next pages.

Each book page becomes one record in the `books` Dataset, exported to `books.json` when the crawl ends.
"""

import re
from decimal import Decimal
from pathlib import Path
from typing import Annotated

from pydantic import BaseModel, Field, PlainSerializer

from goodfetch_foxhound import (
    Crawl,
    CrawlContext,
    CrawlRequestBlocked,
    CrawlRequestFailed,
    CrawlRequestSkipped,
    Foxhound,
    LinkInfo,
    seed,
)

START_URL = "https://books.toscrape.com/"
EXPORT_PATH = Path("books.json")

RATING_WORDS: dict[str, int] = {"One": 1, "Two": 2, "Three": 3, "Four": 4, "Five": 5}


def price_as_number(value: Decimal) -> float:
    return float(value)


# A decimal amount of pounds: a Decimal in the program, a JSON number in the export.
Price = Annotated[Decimal, PlainSerializer(price_as_number, return_type=float, when_used="json")]


class CategoryPayload(BaseModel):
    category: str


class BookPayload(BaseModel):
    category: str


class Book(BaseModel):
    url: str
    title: str
    price: Price
    stock: int = Field(ge=0)
    rating: int = Field(ge=1, le=5)
    category: str


crawl = Crawl()
home = crawl.label("home")
category = crawl.label("category", payload=CategoryPayload)
book = crawl.label("book", payload=BookPayload)
books = crawl.dataset("books", schema=Book, key=["url"])


def category_of(link: LinkInfo) -> CategoryPayload:
    return CategoryPayload(category=link.text.strip())


def parse_price(text: str) -> Decimal:
    match = re.search(r"\d+(?:\.\d+)?", text)
    if match is None:
        raise ValueError(f"no price in {text!r}")
    return Decimal(match.group(0))


def parse_stock(text: str) -> int:
    match = re.search(r"\((\d+) available\)", text)
    if match is None:
        raise ValueError(f"no stock count in {text!r}")
    return int(match.group(1))


def parse_rating(class_attribute: str | None) -> int:
    for word in (class_attribute or "").split():
        if word in RATING_WORDS:
            return RATING_WORDS[word]
    raise ValueError(f"no star rating in {class_attribute!r}")


@crawl.handle(home)
def handle_home(ctx: CrawlContext[None]) -> None:
    # The side list's nested links are the categories; the top-level "Books" link is not nested.
    ctx.enqueue_links(ctx.page.locator("ul.nav-list ul a"), category, category_of)


@crawl.handle(category)
def handle_category(ctx: CrawlContext[CategoryPayload]) -> None:
    name = ctx.request.payload.category
    page = ctx.page
    ctx.enqueue_links(page.locator("article.product_pod h3 a"), book, BookPayload(category=name))
    next_links = page.locator("li.next a")
    if next_links.count() > 0:
        ctx.enqueue_links(next_links, category, CategoryPayload(category=name))


@crawl.handle(book)
def handle_book(ctx: CrawlContext[BookPayload]) -> None:
    page = ctx.page
    if page.locator("div.product_main").count() == 0:
        ctx.skip("not a book page: no div.product_main")
    title = page.locator("div.product_main h1").text_content().strip()
    price = parse_price(page.locator("p.price_color").text_content())
    stock = parse_stock(page.locator("p.instock.availability").text_content())
    rating = parse_rating(page.locator("p.star-rating").get_attribute("class"))
    ctx.push(
        books,
        Book(
            url=page.url,
            title=title,
            price=price,
            stock=stock,
            rating=rating,
            category=ctx.request.payload.category,
        ),
    )


def main() -> None:
    with Foxhound.connect() as fox:
        run = fox.start_crawl(
            crawl,
            seeds=[seed(START_URL, home)],
            settings={"crawl": {"scope": {"strategy": "same_hostname"}}},
        )
        report = run.finish()
        totals = report.requests
        print(
            f"pages: {totals.succeeded} succeeded, {totals.failed} failed, "
            f"{totals.skipped} skipped, {totals.blocked} blocked"
        )
        for outcome in run.outcomes(kinds=["failed", "skipped", "blocked"]):
            if isinstance(outcome, CrawlRequestFailed):
                print(f"failed {outcome.request.url}: {outcome.error.code}: {outcome.error.message}")
            elif isinstance(outcome, CrawlRequestSkipped):
                print(f"skipped {outcome.request.url}: {outcome.reason}: {outcome.message}")
            elif isinstance(outcome, CrawlRequestBlocked):
                print(f"blocked {outcome.request.url}: {outcome.block_cause}")
        export = run.export(books, format="json")
        export.save_as(EXPORT_PATH)
        print(f"exported {export.records} records to {EXPORT_PATH}")


if __name__ == "__main__":
    main()
