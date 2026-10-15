"""Golden scenario 10 on Scrapy 2.19.0, with pytest: scenario 05's spider tested with no network.

The book callback runs on an `HtmlResponse` built from a saved page. The whole crawl replays the HTTP cache that one
live run filled (`HTTPCACHE_ENABLED` with this `HTTPCACHE_DIR`), committed beside the tests. Run it with
`pytest s10_offline_crawl_tests_scrapy.py`.

Scrapy's contracts (`@url`, `@cb_kwargs`, `@scrapes` in a callback's docstring, run by `scrapy check`) fetch the live
URL they name, so they cannot serve a test with no network. Scrapy has no AI extraction, so scenario 08's test has no
counterpart here.
"""

from decimal import Decimal
from pathlib import Path

from scrapy.crawler import AsyncCrawlerProcess
from scrapy.http import HtmlResponse

from s05_two_page_crawl_scrapy import BooksSpider

BOOK = "https://books.toscrape.com/catalogue/a-light-in-the-attic_1000/index.html"
PAGE = Path("tests/fixtures/a-light-in-the-attic.html")
REPLAY = {
    "HTTPCACHE_ENABLED": True,
    "HTTPCACHE_DIR": str(Path("tests/httpcache").resolve()),  # a relative one is under .scrapy/
    # A request the cache lacks is dropped with IgnoreRequest instead of fetched; without this, it is fetched.
    "HTTPCACHE_IGNORE_MISSING": True,
    # HTTPCACHE_EXPIRATION_SECS is 0 by default: the cache never expires.
}


def test_book_page_gives_its_book_with_the_category_passed_to_it() -> None:
    response = HtmlResponse(BOOK, body=PAGE.read_bytes(), encoding="utf-8")
    items = list(BooksSpider().parse_book(response, category="Poetry"))
    assert items == [
        {
            "title": "A Light in the Attic",
            "price": Decimal("51.77"),
            "in_stock": 22,
            "rating": 3,
            "url": BOOK,
            "category": "Poetry",
        }
    ]


def test_crawl_collects_every_book_from_the_cache() -> None:
    # The reactor starts once per process: one crawl per test session.
    process = AsyncCrawlerProcess(settings=REPLAY)
    crawler = process.create_crawler(BooksSpider)
    process.crawl(crawler)
    process.start()
    # A dropped miss is only counted here; its URL is in the log's IgnoreRequest message.
    assert crawler.stats.get_value("httpcache/miss", 0) == 0
    assert crawler.stats.get_value("item_scraped_count") == 1000
