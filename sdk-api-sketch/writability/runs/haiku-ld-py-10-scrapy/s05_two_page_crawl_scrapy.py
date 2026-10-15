"""Golden scenario 05 on Scrapy 2.19.0: a Spider with callbacks, `cb_kwargs` and a feed export. Scenario 07's program
imports the spider; this one runs only as a script (`python s05_two_page_crawl_scrapy.py`)."""

import re
from collections import Counter
from collections.abc import Iterator
from decimal import Decimal
from typing import Any

import scrapy
from scrapy.crawler import CrawlerProcess
from scrapy.exceptions import IgnoreRequest
from scrapy.spidermiddlewares.httperror import HttpError
from twisted.python.failure import Failure

RATINGS = ("One", "Two", "Three", "Four", "Five")
# The politeness `scrapy startproject` writes into a project's settings.py.
POLITE = {
    "ROBOTSTXT_OBEY": True,
    "CONCURRENT_REQUESTS_PER_DOMAIN": 1,
    "DOWNLOAD_DELAY": 1,
}


class BooksSpider(scrapy.Spider):
    name = "books"
    allowed_domains = ["books.toscrape.com"]
    start_urls = ["https://books.toscrape.com/"]
    custom_settings = {
        **POLITE,
        "FEEDS": {
            "books.json": {"format": "json", "encoding": "utf8", "overwrite": True}
        },
    }

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        super().__init__(*args, **kwargs)
        self.succeeded = 0
        self.failures: list[tuple[str, str]] = []
        self.skips: list[tuple[str, str]] = []

    def parse(self, response) -> Iterator[scrapy.Request]:
        self.succeeded += 1
        # The nested list leaves out the top-level "Books".
        for link in response.css(".side_categories ul ul a"):
            name = (link.css("::text").get() or "").strip()
            yield response.follow(
                link,
                self.parse_category,
                cb_kwargs={"category": name},
                errback=self.errback,
            )

    def parse_category(self, response, category: str) -> Iterator[scrapy.Request]:
        self.succeeded += 1
        kwargs = {"cb_kwargs": {"category": category}, "errback": self.errback}
        yield from response.follow_all(
            css="article.product_pod h3 a", callback=self.parse_book, **kwargs
        )
        yield from response.follow_all(
            css="li.next a", callback=self.parse_category, **kwargs
        )

    def parse_book(self, response, category: str) -> Iterator[dict[str, Any]]:
        main = response.css(".product_main")
        price = re.search(r"£(\d+\.\d{2})", main.css(".price_color::text").get(""))
        stock = re.search(
            r"\((\d+) available\)", " ".join(main.css(".availability::text").getall())
        )
        stars = main.css(".star-rating::attr(class)").get("")
        rating = next((n for n, word in enumerate(RATINGS, 1) if word in stars), None)
        if price is None or stock is None or rating is None:
            self.skips.append((response.url, "no price, stock or rating on the page"))
            return
        self.succeeded += 1
        yield {
            "title": main.css("h1::text").get(),
            "price": Decimal(price[1]),
            "in_stock": int(stock[1]),
            "rating": rating,
            "url": response.url,
            "category": category,
        }

    def errback(self, failure: Failure) -> None:
        url, error = failure.request.url, failure.value  # type: ignore[attr-defined]
        if isinstance(error, HttpError):
            self.failures.append((url, f"HTTP {error.response.status}"))
        elif isinstance(error, IgnoreRequest):
            self.skips.append((url, str(error)))  # robots.txt, offsite
        else:
            self.failures.append((url, repr(error)))

    def closed(self, reason: str) -> None:
        for url, why in self.failures:
            print(f"failed {url}: {why}")
        for url, why in self.skips:
            print(f"skipped {url}: {why}")
        print(
            f"{self.succeeded} succeeded, {len(self.failures)} failed, {len(self.skips)} skipped ({reason})"
        )
        print("by reason", Counter(why for _, why in self.failures + self.skips))


if __name__ == "__main__":
    process = CrawlerProcess()
    process.crawl(BooksSpider)
    process.start()
