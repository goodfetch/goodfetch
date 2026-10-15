"""Books service: `GET /books?category=<name>` crawls one category of https://books.toscrape.com/ and answers its books.

Each call runs its own crawl on its own connection, so concurrent and repeated calls do not share state. When the caller
disconnects, the crawl is stopped, and closing the connection releases the engine's browsers and connections. The same
crawl is available synchronously from a notebook cell through `crawl_books_sync`.
"""

import asyncio
import logging
from collections.abc import Awaitable, Callable
from contextlib import suppress

from fastapi import FastAPI, Request
from pydantic import BaseModel

from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncCrawlContext,
    AsyncCrawlRun,
    AsyncFoxhound,
    Crawl,
    CrawlContext,
    Foxhound,
    seed,
)

HOME_URL = "https://books.toscrape.com/"
_POLL_SECONDS = 0.2
_SERVICE_LOG = logging.getLogger("books_service")


class Book(BaseModel):
    title: str
    price: str


class CategoryRequest(BaseModel):
    category: str


class _ForwardToServiceLogger(logging.Handler):
    def __init__(self, target: logging.Logger) -> None:
        super().__init__()
        self._target = target

    def emit(self, record: logging.LogRecord) -> None:
        self._target.handle(record)


def route_library_logs() -> None:
    """Sends the library's `goodfetch` loggers to the service's own logger. Nothing is written to stdout."""
    library = logging.getLogger("goodfetch")
    if any(isinstance(handler, _ForwardToServiceLogger) for handler in library.handlers):
        return
    library.addHandler(_ForwardToServiceLogger(_SERVICE_LOG))
    library.propagate = False


# The asynchronous crawl, used by the service.

_ASYNC_CRAWL = AsyncCrawl()
async_home = _ASYNC_CRAWL.label("home", payload=CategoryRequest)
async_listing = _ASYNC_CRAWL.label("listing")
async_books = _ASYNC_CRAWL.dataset("books", schema=Book)


@_ASYNC_CRAWL.handle(async_home)
async def async_home_handler(ctx: AsyncCrawlContext[CategoryRequest]) -> None:
    links = ctx.page.get_by_role("link", name=ctx.request.payload.category, exact=True)
    await ctx.enqueue_links(links, async_listing, limit=1)


@_ASYNC_CRAWL.handle(async_listing)
async def async_listing_handler(ctx: AsyncCrawlContext[None]) -> None:
    for item in await ctx.page.locator("article.product_pod").all():
        title = await item.locator("h3 a").get_attribute("title")
        price = await item.locator("p.price_color").text_content()
        if title is not None:
            await ctx.push(async_books, Book(title=title, price=price.strip()))
    await ctx.enqueue_links(ctx.page.locator("li.next a"), async_listing, limit=1)


# The synchronous crawl, used from a notebook cell.

_SYNC_CRAWL = Crawl()
sync_home = _SYNC_CRAWL.label("home", payload=CategoryRequest)
sync_listing = _SYNC_CRAWL.label("listing")
sync_books = _SYNC_CRAWL.dataset("books", schema=Book)


@_SYNC_CRAWL.handle(sync_home)
def sync_home_handler(ctx: CrawlContext[CategoryRequest]) -> None:
    links = ctx.page.get_by_role("link", name=ctx.request.payload.category, exact=True)
    ctx.enqueue_links(links, sync_listing, limit=1)


@_SYNC_CRAWL.handle(sync_listing)
def sync_listing_handler(ctx: CrawlContext[None]) -> None:
    for item in ctx.page.locator("article.product_pod").all():
        title = item.locator("h3 a").get_attribute("title")
        price = item.locator("p.price_color").text_content()
        if title is not None:
            ctx.push(sync_books, Book(title=title, price=price.strip()))
    ctx.enqueue_links(ctx.page.locator("li.next a"), sync_listing, limit=1)


async def _stop_when_client_gone(run: AsyncCrawlRun, client_gone: Callable[[], Awaitable[bool]]) -> None:
    while not await client_gone():
        await asyncio.sleep(_POLL_SECONDS)
    _SERVICE_LOG.info("caller disconnected; stopping the crawl")
    try:
        await run.stop()
    except Exception:
        _SERVICE_LOG.exception("stopping the crawl after the caller disconnected failed")


async def crawl_books(category: str, *, client_gone: Callable[[], Awaitable[bool]] | None = None) -> list[Book]:
    """Crawls one category and returns its books. `client_gone` tells whether the caller has gone; then the crawl stops."""
    books: list[Book] = []
    async with AsyncFoxhound.connect() as fox:
        run = await fox.start_crawl(
            _ASYNC_CRAWL,
            seeds=[seed(HOME_URL, async_home, CategoryRequest(category=category))],
        )
        async with run:
            watcher = asyncio.create_task(_stop_when_client_gone(run, client_gone)) if client_gone is not None else None
            try:
                async for record in run.records(async_books):
                    books.append(record.value)
                await run.finish()
            except BaseException:
                with suppress(Exception):
                    await run.stop()
                raise
            finally:
                if watcher is not None:
                    watcher.cancel()
    return books


def crawl_books_sync(category: str) -> list[Book]:
    """The same crawl for a notebook cell or a plain script: blocking, and safe to call from any thread."""
    books: list[Book] = []
    with Foxhound.connect() as fox:
        run = fox.start_crawl(
            _SYNC_CRAWL,
            seeds=[seed(HOME_URL, sync_home, CategoryRequest(category=category))],
        )
        with run:
            try:
                for record in run.records(sync_books):
                    books.append(record.value)
                run.finish()
            except BaseException:
                with suppress(Exception):
                    run.stop()
                raise
    return books


app = FastAPI()


@app.get("/books")
async def books_endpoint(category: str, request: Request) -> list[Book]:
    return await crawl_books(category, client_gone=request.is_disconnected)


route_library_logs()
