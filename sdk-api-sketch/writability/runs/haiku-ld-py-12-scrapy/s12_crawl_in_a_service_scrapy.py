"""A small web service that crawls one category of https://books.toscrape.com/.

GET /books?category=<name> crawls that category (following its "next" pages)
and answers with its books as JSON: {"category": ..., "books": [{"title", "price"}, ...]}.

Design:
- Scrapy runs in reactorless mode (TWISTED_REACTOR_ENABLED=False) on the
  asyncio loop that FastAPI and uvicorn already run, through AsyncCrawlerRunner.
- Every call builds its own runner, crawler and spider, so concurrent calls and
  repeated calls share no state.
- While the crawl runs, the endpoint polls request.is_disconnected(). When the
  caller is gone the crawler is stopped, which closes the engine, its slots and
  the aiohttp download handlers before the call returns.
- Scrapy logs through the standard logging module. This file installs no Scrapy
  root handler and writes nothing to standard output, so the service's logging
  configuration (set up in __main__) receives every record.

Notebook use (top-level await in a cell, the notebook already runs an asyncio loop):

    from s12_crawl_in_a_service_scrapy import crawl_category
    result = await crawl_category("Mystery")
    result.books

Run the service with: uvicorn s12_crawl_in_a_service_scrapy:app
"""

from __future__ import annotations

import asyncio
import logging
from collections.abc import AsyncIterator, Awaitable, Callable, Iterator
from dataclasses import asdict, dataclass, field
from typing import Any

from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse, Response
from scrapy import Request as ScrapyRequest
from scrapy import Spider
from scrapy.crawler import AsyncCrawlerRunner
from scrapy.http import Response as ScrapyResponse

HOME_URL = "https://books.toscrape.com/"
POLL_SECONDS = 0.2

# No Twisted reactor: Scrapy runs on the asyncio loop of the caller (uvicorn or a notebook kernel).
CRAWL_SETTINGS: dict[str, Any] = {
    "TWISTED_REACTOR_ENABLED": False,
    "ROBOTSTXT_OBEY": True,
    "COOKIES_ENABLED": False,
}

log = logging.getLogger(__name__)


@dataclass(frozen=True)
class Book:
    title: str
    price: str


@dataclass
class CategoryBooks:
    category: str
    found: bool = False
    books: list[Book] = field(default_factory=list)


class CrawlAbandoned(Exception):
    """The caller disconnected before the crawl finished; the crawl was stopped."""


def _normalise(name: str) -> str:
    return " ".join(name.replace("-", " ").replace("_", " ").split()).casefold()


class CategorySpider(Spider):
    name = "books_category"

    def __init__(self, category: str, result: CategoryBooks, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self.category = category
        self.result = result

    async def start(self) -> AsyncIterator[ScrapyRequest]:
        yield ScrapyRequest(HOME_URL, callback=self.parse_home)

    def parse_home(self, response: ScrapyResponse) -> Iterator[ScrapyRequest]:
        wanted = _normalise(self.category)
        for link in response.css("ul.nav-list a"):
            label = " ".join(link.css("::text").getall())
            href = link.css("::attr(href)").get()
            if _normalise(label) == wanted and href is not None:
                self.result.found = True
                yield response.follow(href, callback=self.parse_listing)
                return
        log.info("No category named %r on %s", self.category, HOME_URL)

    def parse_listing(self, response: ScrapyResponse) -> Iterator[ScrapyRequest]:
        for pod in response.css("article.product_pod"):
            title = pod.css("h3 a::attr(title)").get()
            price = pod.css("p.price_color::text").get()
            if title is not None and price is not None:
                self.result.books.append(Book(title=title, price=price.strip()))
        next_page = response.css("li.next a::attr(href)").get()
        if next_page is not None:
            yield response.follow(next_page, callback=self.parse_listing)


async def crawl_category(
    category: str,
    client_gone: Callable[[], Awaitable[bool]] | None = None,
) -> CategoryBooks:
    """Crawl one category and return its books.

    client_gone is polled while the crawl runs. When it reports True the crawl
    is stopped and fully closed, then CrawlAbandoned is raised.
    """
    runner = AsyncCrawlerRunner(CRAWL_SETTINGS)
    crawler = runner.create_crawler(CategorySpider)
    result = CategoryBooks(category=category)
    crawl = runner.crawl(crawler, category=category, result=result)
    try:
        while True:
            done, _ = await asyncio.wait({crawl}, timeout=POLL_SECONDS)
            if done:
                crawl.result()
                break
            if client_gone is not None and await client_gone():
                log.info("Caller disconnected; stopping the crawl of %r", category)
                raise CrawlAbandoned(category)
    finally:
        if not crawl.done():
            await crawler.stop_async()
            await asyncio.wait({crawl})
            stop_error = crawl.exception()
            if stop_error is not None:
                log.warning("Crawl of %r failed while stopping: %s", category, stop_error)
    log.info("Crawled %r: %d books", category, len(result.books))
    return result


app = FastAPI(title="Books crawl service")


@app.get("/books")
async def list_books(category: str, request: Request) -> Response:
    try:
        result = await crawl_category(category, client_gone=request.is_disconnected)
    except CrawlAbandoned:
        # 499 is the conventional "client closed request" status; nobody is left to read it.
        return Response(status_code=499)
    if not result.found:
        return JSONResponse({"detail": f"unknown category: {category}"}, status_code=404)
    return JSONResponse(
        {"category": category, "books": [asdict(book) for book in result.books]}
    )


if __name__ == "__main__":
    import uvicorn

    logging.basicConfig(level=logging.INFO)
    uvicorn.run(app, host="127.0.0.1", port=8000)
