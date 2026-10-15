"""Golden scenario 12, "Run a crawl inside a web service": the leader's program, Scrapy 2.19.0 run from a script with
`AsyncCrawlerRunner` inside FastAPI (Python). Run it with `uvicorn s12_crawl_in_a_service_scrapy:app`."""

import asyncio
import logging
from collections.abc import Iterator
from typing import Any

import scrapy
from fastapi import FastAPI, HTTPException, Request
from scrapy import signals
from scrapy.crawler import AsyncCrawlerRunner, Crawler
from scrapy.exceptions import CloseSpider
from scrapy.http import Response, TextResponse

# Scrapy logs to `logging` loggers under `scrapy`; a runner never calls `configure_logging()`, so the service's own
# logging configuration (uvicorn's here) decides where they go.
log = logging.getLogger("bookshop")


class CategorySpider(scrapy.Spider):
    name = "category"
    start_urls = ["https://books.toscrape.com/"]

    def __init__(self, category: str, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self.category = category

    def parse(self, response: TextResponse) -> Iterator[Any]:
        href = response.xpath(
            '//div[@class="side_categories"]/ul/li/ul/li/a[normalize-space()=$name]/@href',
            name=self.category,
        ).get()
        if href is None:
            raise CloseSpider("no_such_category")
        yield response.follow(href, self.parse_category)

    def parse_category(self, response: Response) -> Iterator[Any]:
        for book in response.css("article.product_pod"):
            yield {
                "title": book.css("h3 a::attr(title)").get(),
                "price": book.css(".price_color::text").get(),
            }
        next_pages = response.css("li.next a::attr(href)").getall()
        yield from response.follow_all(next_pages, callback=self.parse_category)


# Without a Twisted reactor (experimental since 2.15), Scrapy runs on the service's own asyncio loop, so a second crawl
# in the process works and no reactor has to be started or stopped.
runner = AsyncCrawlerRunner({"TWISTED_REACTOR_ENABLED": False})


async def books_in(category: str, crawler: Crawler) -> list[dict[str, Any]] | None:
    """The category's books, or None when the site has no such category."""
    items: list[dict[str, Any]] = []

    def collect(item: dict[str, Any]) -> None:
        items.append(item)

    crawler.signals.connect(collect, signal=signals.item_scraped)
    await runner.crawl(crawler, category=category)
    if (
        crawler.stats is not None
        and crawler.stats.get_value("finish_reason") == "no_such_category"
    ):
        return None
    return items


app = FastAPI()


async def stop_when_gone(request: Request, crawler: Crawler) -> None:
    """Starlette does not cancel an endpoint whose caller disconnects: this watches for it."""
    while (await request.receive())["type"] != "http.disconnect":
        pass
    await crawler.stop_async()  # a graceful stop: in-flight requests finish first


@app.get("/books")
async def get_books(category: str, request: Request) -> list[dict[str, Any]]:
    crawler = runner.create_crawler(CategorySpider)
    watcher = asyncio.create_task(stop_when_gone(request, crawler))
    try:
        found = await books_in(category, crawler)
    finally:
        watcher.cancel()
    if found is None:
        raise HTTPException(status_code=404, detail=f"no category named {category!r}")
    log.info("crawled %s: %d books", category, len(found))
    return found


async def notebook_cell() -> list[dict[str, Any]] | None:
    """The same crawl from a notebook cell, written as the cell's body, as Scrapy's docs show it: `await` on the
    notebook's running loop, with no reactor."""
    return await books_in("Travel", runner.create_crawler(CategorySpider))
