"""Crawl books.toscrape.com with Scrapy 2.19.

From the home page the spider follows every category link in the side list except the
top-level "Books" link. Each category page yields its book links and its "next" page link.
Each book page yields one BookItem; the category name comes from the category page that linked
to it. Items are stored in the named dataset DATASET_NAME and exported to books.json when the
crawl ends. The crawl logs how many pages succeeded, failed or were skipped, with the reason for
each failed or skipped page.
"""

import json
import re
from collections.abc import AsyncIterator, Callable, Iterator
from dataclasses import asdict, dataclass
from functools import partial
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

from scrapy import Request, Spider
from scrapy.crawler import CrawlerProcess
from scrapy.exceptions import IgnoreRequest
from scrapy.http import Response
from scrapy.spidermiddlewares.httperror import HttpError
from twisted.python.failure import Failure

START_URL = "https://books.toscrape.com/"
ALLOWED_HOST = "books.toscrape.com"
DATASET_NAME = "books"
OUTPUT_PATH = Path(__file__).with_name("books.json")

STAR_WORDS = {"One": 1, "Two": 2, "Three": 3, "Four": 4, "Five": 5}
PRICE_PATTERN = re.compile(r"£\s*(\d+(?:\.\d+)?)")
STOCK_PATTERN = re.compile(r"\((\d+) available\)")


class ParseError(Exception):
    """A book page lacks a field the item needs."""


@dataclass(frozen=True)
class BookItem:
    title: str
    price: float
    in_stock: int
    rating: int
    url: str
    category: str


def _required_text(response: Response, selector: str, field: str) -> str:
    value = response.css(selector).get()
    if value is None or not value.strip():
        raise ParseError(f"no {field} found")
    return value.strip()


def extract_book(response: Response, category: str) -> BookItem:
    title = _required_text(response, "div.product_main h1::text", "title")

    price_text = _required_text(response, "p.price_color::text", "price")
    price_match = PRICE_PATTERN.search(price_text)
    if price_match is None:
        raise ParseError(f"price {price_text!r} is not a pound amount")
    price = float(price_match.group(1))

    availability = " ".join(response.css("p.availability ::text").getall())
    stock_match = STOCK_PATTERN.search(availability)
    if stock_match is not None:
        in_stock = int(stock_match.group(1))
    elif "out of stock" in availability.lower():
        in_stock = 0
    else:
        raise ParseError(f"availability {availability.strip()!r} has no stock count")

    rating_classes = _required_text(response, "p.star-rating::attr(class)", "star rating").split()
    rating_word = next((name for name in rating_classes if name in STAR_WORDS), None)
    if rating_word is None:
        raise ParseError(f"star rating classes {rating_classes!r} name no rating")

    return BookItem(
        title=title,
        price=price,
        in_stock=in_stock,
        rating=STAR_WORDS[rating_word],
        url=response.url,
        category=category,
    )


class BooksSpider(Spider):
    name = "books"
    allowed_domains = [ALLOWED_HOST]

    def __init__(self, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self.datasets: dict[str, list[BookItem]] = {DATASET_NAME: []}
        self.requested: set[str] = {START_URL}
        self.succeeded = 0
        self.failed: list[tuple[str, str]] = []
        self.skipped: list[tuple[str, str]] = []

    async def start(self) -> AsyncIterator[Request]:
        yield Request(START_URL, callback=self.parse_home, errback=partial(self.on_error, START_URL))

    def parse_home(self, response: Response) -> Iterator[Request]:
        self.succeeded += 1
        for link in response.css(".side_categories ul.nav-list ul a"):
            href = link.attrib.get("href")
            name = " ".join(link.css("::text").getall()).strip()
            if not href or not name:
                continue
            request = self._follow(response, href, self.parse_category, name)
            if request is not None:
                yield request

    def parse_category(self, response: Response, category: str) -> Iterator[Request]:
        self.succeeded += 1
        for href in response.css("article.product_pod h3 a::attr(href)").getall():
            request = self._follow(response, href, self.parse_book, category)
            if request is not None:
                yield request
        next_href = response.css("li.next a::attr(href)").get()
        if next_href:
            request = self._follow(response, next_href, self.parse_category, category)
            if request is not None:
                yield request

    def parse_book(self, response: Response, category: str) -> Iterator[BookItem]:
        try:
            item = extract_book(response, category)
        except ParseError as exc:
            self.failed.append((response.url, f"book page could not be parsed: {exc}"))
            return
        self.succeeded += 1
        self.datasets[DATASET_NAME].append(item)
        yield item

    def on_error(self, url: str, failure: Failure) -> None:
        error = failure.value
        if isinstance(error, HttpError):
            self.failed.append((url, f"HTTP {error.response.status} response"))
        elif isinstance(error, IgnoreRequest):
            self.skipped.append((url, f"ignored by Scrapy: {error}"))
        else:
            self.failed.append((url, failure.getErrorMessage() or failure.type.__name__))

    def closed(self, reason: str) -> None:
        self.logger.info(
            "Crawl closed (%s): %d pages succeeded, %d failed, %d skipped",
            reason,
            self.succeeded,
            len(self.failed),
            len(self.skipped),
        )
        for url, why in self.failed:
            self.logger.warning("Failed page %s: %s", url, why)
        for url, why in self.skipped:
            self.logger.info("Skipped page %s: %s", url, why)

        items = self.datasets[DATASET_NAME]
        OUTPUT_PATH.write_text(
            json.dumps([asdict(item) for item in items], indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
        self.logger.info("Exported %d items from dataset %r to %s", len(items), DATASET_NAME, OUTPUT_PATH)

    def _follow(
        self,
        response: Response,
        href: str,
        callback: Callable[..., Any],
        category: str,
    ) -> Request | None:
        url = response.urljoin(href)
        if urlsplit(url).hostname != ALLOWED_HOST:
            self.skipped.append((url, f"off-site: host is not {ALLOWED_HOST}"))
            return None
        if url in self.requested:
            self.skipped.append((url, "duplicate: this URL was already requested"))
            return None
        self.requested.add(url)
        return Request(
            url,
            callback=callback,
            errback=partial(self.on_error, url),
            cb_kwargs={"category": category},
        )


def main() -> None:
    process = CrawlerProcess(settings={"LOG_LEVEL": "INFO"})
    process.crawl(BooksSpider)
    process.start()


if __name__ == "__main__":
    main()
