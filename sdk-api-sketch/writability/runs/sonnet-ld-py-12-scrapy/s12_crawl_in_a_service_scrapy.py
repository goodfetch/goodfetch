"""A small service that crawls one category of https://books.toscrape.com/.

GET /books?category=<name> crawls that category (following its "next" pages)
and answers with the books as JSON: [{"title": ..., "price": ...}, ...].

Design notes
- Each call builds its own Scrapy ``Crawler`` (its own spider, stats, engine and
  HTTP connections), so two calls at the same time share nothing and a call
  later works the same as the first one.
- Scrapy runs without a Twisted reactor (``TWISTED_REACTOR_ENABLED=False``) on
  the event loop that is already running (uvicorn's, or a notebook's), with its
  aiohttp download handler.
- If the caller disconnects, the crawler is stopped gracefully; the engine
  closes its downloader, which closes the HTTP connections.
- Scrapy installs no handler on the root logger here, so its records propagate
  to the service's own logging, which writes to standard error. Nothing is
  printed to standard output.

Same crawl from a notebook cell (Python), where a loop is already running:

    from s12_crawl_in_a_service_scrapy import crawl_category
    books = await crawl_category("Travel")
    books

From a plain Python script:

    import asyncio
    from s12_crawl_in_a_service_scrapy import crawl_category
    print_books = asyncio.run(crawl_category("Travel"))

(A TypeScript or Rust script would call the running service over HTTP:
GET http://127.0.0.1:8000/books?category=Travel.)
"""

from __future__ import annotations

import asyncio
import logging
import sys
from collections.abc import AsyncIterator, Awaitable
from typing import Any, TypedDict

import uvicorn
from fastapi import FastAPI, HTTPException, Query, Request
from fastapi.responses import JSONResponse, Response
from scrapy import Request as ScrapyRequest
from scrapy import Spider
from scrapy.crawler import Crawler
from scrapy.http import Response as ScrapyResponse
from scrapy.http import TextResponse

BASE_URL = "https://books.toscrape.com/"

logger = logging.getLogger("books_service")


class Book(TypedDict):
    title: str
    price: str


class CategoryNotFound(Exception):
    """The site has no category with that name."""


class CallerDisconnected(Exception):
    """The caller went away before the crawl ended."""


class BooksSpider(Spider):
    name = "books"

    def __init__(self, category: str, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self.category = category.strip().casefold()
        self.books: list[Book] = []
        self.category_found = False

    async def start(self) -> AsyncIterator[Any]:
        yield ScrapyRequest(BASE_URL, callback=self.parse_home)

    def parse_home(self, response: ScrapyResponse) -> Any:
        if not isinstance(response, TextResponse):
            return
        for link in response.css("div.side_categories ul li ul li a"):
            label = " ".join(link.css("::text").getall()).strip()
            href = link.attrib.get("href")
            if href and label.casefold() == self.category:
                self.category_found = True
                yield response.follow(href, callback=self.parse_listing)
                return

    def parse_listing(self, response: ScrapyResponse) -> Any:
        if not isinstance(response, TextResponse):
            return
        for pod in response.css("article.product_pod"):
            title = pod.css("h3 a::attr(title)").get()
            price = pod.css("p.price_color::text").get()
            if title is not None and price is not None:
                self.books.append({"title": title, "price": price.strip()})
        next_href = response.css("li.next a::attr(href)").get()
        if next_href:
            yield response.follow(next_href, callback=self.parse_listing)


def _settings() -> dict[str, Any]:
    return {
        "TWISTED_REACTOR_ENABLED": False,
        "TELNETCONSOLE_ENABLED": False,
        "LOG_LEVEL": "INFO",
        "COOKIES_ENABLED": False,
        "CONCURRENT_REQUESTS": 8,
        "DOWNLOAD_TIMEOUT": 30,
        "RETRY_TIMES": 2,
        "ROBOTSTXT_OBEY": False,
        "USER_AGENT": "books-service (scrapy)",
    }


async def crawl_category(
    category: str, disconnected: Awaitable[Any] | None = None
) -> list[Book]:
    """Crawl one category and return its books.

    ``disconnected``, when given, is an awaitable that completes when the
    caller goes away; the crawl is then stopped and CallerDisconnected raised.
    """
    crawler = Crawler(BooksSpider, _settings())
    crawl = asyncio.ensure_future(crawler.crawl_async(category=category))
    watch = asyncio.ensure_future(disconnected) if disconnected is not None else None
    try:
        waiting: set[asyncio.Future[Any]] = {crawl}
        if watch is not None:
            waiting.add(watch)
        await asyncio.wait(waiting, return_when=asyncio.FIRST_COMPLETED)
        if not crawl.done():
            raise CallerDisconnected(category)
        crawl.result()
    finally:
        if watch is not None and not watch.done():
            watch.cancel()
        if not crawl.done():
            logger.info("stopping the crawl of %r", category)
            try:
                await crawler.stop_async()
                await asyncio.wait({crawl}, timeout=10)
            finally:
                if not crawl.done():
                    crawl.cancel()
                    await asyncio.wait({crawl}, timeout=5)
        if crawl.done() and not crawl.cancelled():
            crawl.exception()  # mark any error as retrieved

    spider = crawler.spider
    if not isinstance(spider, BooksSpider):
        raise RuntimeError("the spider did not start")
    if not spider.category_found:
        raise CategoryNotFound(category)
    return spider.books


async def _wait_disconnected(request: Request) -> None:
    while not await request.is_disconnected():
        await asyncio.sleep(0.1)


app = FastAPI()


@app.get("/books")
async def books(
    request: Request, category: str = Query(min_length=1)
) -> Response:
    logger.info("crawl of category %r started", category)
    try:
        result = await crawl_category(category, _wait_disconnected(request))
    except CallerDisconnected:
        logger.info("caller disconnected, crawl of %r stopped", category)
        return Response(status_code=499)
    except CategoryNotFound:
        raise HTTPException(status_code=404, detail=f"no category {category!r}")
    logger.info("crawl of category %r done: %d books", category, len(result))
    return JSONResponse(result)


def configure_logging() -> None:
    handler = logging.StreamHandler(sys.stderr)
    handler.setFormatter(
        logging.Formatter("%(asctime)s [%(name)s] %(levelname)s: %(message)s")
    )
    root = logging.getLogger()
    if not root.handlers:
        root.addHandler(handler)
    root.setLevel(logging.INFO)


def main() -> None:
    configure_logging()
    uvicorn.run(app, host="127.0.0.1", port=8000, log_config=None, access_log=False)


if __name__ == "__main__":
    main()
