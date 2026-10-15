"""Crawl books.toscrape.com: home page -> category pages -> book pages.

Items go into a named dataset ("books") that is exported to a JSON file when the
crawl ends; the run ends with a report of succeeded, failed and skipped pages.
"""

from __future__ import annotations

import json
import re
from collections import Counter
from collections.abc import AsyncIterator, Iterator
from decimal import Decimal
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

import scrapy
from scrapy import signals
from scrapy.crawler import Crawler, CrawlerProcess
from scrapy.exceptions import IgnoreRequest
from scrapy.http import Request, Response, TextResponse
from scrapy.spidermiddlewares.httperror import HttpError
from twisted.python.failure import Failure
from typing_extensions import Self

SITE = "books.toscrape.com"
START_URL = f"https://{SITE}/"
DATASET_NAME = "books"
OUTPUT_FILE = Path(__file__).with_name(f"{DATASET_NAME}.json")

RATINGS = {"One": 1, "Two": 2, "Three": 3, "Four": 4, "Five": 5}

SUCCEEDED = "succeeded"
FAILED = "failed"
SKIPPED = "skipped"


def as_text(response: Response) -> TextResponse:
    if not isinstance(response, TextResponse):
        raise TypeError(f"{response.url} is not a text response")
    return response


def on_site(url: str) -> bool:
    host = urlparse(url).hostname or ""
    return host == SITE


class BooksSpider(scrapy.Spider):
    name = "books"
    allowed_domains = [SITE]
    custom_settings = {
        "ROBOTSTXT_OBEY": True,
        "LOG_LEVEL": "INFO",
    }

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        super().__init__(*args, **kwargs)
        # Named datasets: dataset name -> records.
        self.datasets: dict[str, list[dict[str, Any]]] = {DATASET_NAME: []}
        # URL -> (outcome, reason)
        self.outcomes: dict[str, tuple[str, str]] = {}

    @classmethod
    def from_crawler(cls, crawler: Crawler, *args: Any, **kwargs: Any) -> Self:
        spider = super().from_crawler(crawler, *args, **kwargs)
        crawler.signals.connect(spider.on_spider_error, signal=signals.spider_error)
        crawler.signals.connect(spider.on_request_dropped, signal=signals.request_dropped)
        return spider

    # -- bookkeeping -------------------------------------------------------

    def record(self, url: str, outcome: str, reason: str = "") -> None:
        self.outcomes[url] = (outcome, reason)

    def on_spider_error(self, failure: Failure, response: Response) -> None:
        self.record(response.url, FAILED, f"error while processing: {failure.getErrorMessage()}")

    def on_request_dropped(self, request: Request) -> None:
        self.record(request.url, SKIPPED, "request dropped by the scheduler (duplicate)")

    def on_error(self, failure: Failure) -> None:
        error = failure.value
        if isinstance(error, HttpError):
            self.record(error.response.url, FAILED, f"HTTP status {error.response.status}")
            return
        request: Request | None = getattr(failure, "request", None)
        url = request.url if request is not None else "<unknown>"
        if isinstance(error, IgnoreRequest):
            self.record(url, SKIPPED, f"ignored: {error}")
        else:
            self.record(url, FAILED, f"{type(error).__name__}: {error}")

    # -- crawl -------------------------------------------------------------

    async def start(self) -> AsyncIterator[Any]:
        yield Request(START_URL, callback=self.parse, errback=self.on_error)

    def follow(self, url: str, callback: Any, **cb_kwargs: Any) -> Request | None:
        if not on_site(url):
            self.record(url, SKIPPED, "off-site link")
            return None
        return Request(url, callback=callback, errback=self.on_error, cb_kwargs=cb_kwargs)

    def parse(self, response: Response, **kwargs: Any) -> Iterator[Request]:
        page = as_text(response)
        self.record(page.url, SUCCEEDED)
        links = page.css("div.side_categories ul.nav-list > li > ul > li > a::attr(href)").getall()
        for href in links:
            request = self.follow(page.urljoin(href), self.parse_category)
            if request is not None:
                yield request

    def parse_category(self, response: Response, **kwargs: Any) -> Iterator[Request]:
        page = as_text(response)
        self.record(page.url, SUCCEEDED)
        category = (page.css("div.page-header h1::text").get() or "").strip()
        for href in page.css("article.product_pod h3 a::attr(href)").getall():
            request = self.follow(page.urljoin(href), self.parse_book, category=category)
            if request is not None:
                yield request
        next_href = page.css("li.next a::attr(href)").get()
        if next_href:
            request = self.follow(page.urljoin(next_href), self.parse_category)
            if request is not None:
                yield request

    def parse_book(self, response: Response, category: str, **kwargs: Any) -> Iterator[dict[str, Any]]:
        page = as_text(response)
        self.record(page.url, SUCCEEDED)
        title = (page.css("div.product_main h1::text").get() or "").strip()

        price_text = page.css("p.price_color::text").get() or ""
        price_match = re.search(r"\d+(?:\.\d+)?", price_text)
        if price_match is None:
            raise ValueError(f"no price in {price_text!r}")
        price = Decimal(price_match.group(0))

        stock_text = " ".join(page.css("p.availability::text").getall())
        stock_match = re.search(r"\((\d+) available\)", stock_text)
        in_stock = int(stock_match.group(1)) if stock_match else 0

        rating = 0
        for css_class in (page.css("p.star-rating::attr(class)").get() or "").split():
            if css_class in RATINGS:
                rating = RATINGS[css_class]
        if rating == 0:
            raise ValueError("no star rating")

        item: dict[str, Any] = {
            "title": title,
            "price": price,
            "in_stock": in_stock,
            "rating": rating,
            "url": page.url,
            "category": category,
        }
        self.datasets[DATASET_NAME].append(item)
        yield item

    # -- end of crawl ------------------------------------------------------

    def closed(self, reason: str) -> None:
        records = self.datasets[DATASET_NAME]
        OUTPUT_FILE.write_text(
            json.dumps(records, indent=2, ensure_ascii=False, default=float),
            encoding="utf-8",
        )
        counts = Counter(outcome for outcome, _ in self.outcomes.values())
        print(f"Crawl finished ({reason}); {len(records)} items exported to {OUTPUT_FILE}")
        print(
            f"Pages: {counts[SUCCEEDED]} succeeded, "
            f"{counts[FAILED]} failed, {counts[SKIPPED]} skipped"
        )
        for url, (outcome, why) in sorted(self.outcomes.items()):
            if outcome != SUCCEEDED:
                print(f"  {outcome}: {url} -- {why}")


def main() -> None:
    process = CrawlerProcess()
    process.crawl(BooksSpider)
    process.start()


if __name__ == "__main__":
    main()
