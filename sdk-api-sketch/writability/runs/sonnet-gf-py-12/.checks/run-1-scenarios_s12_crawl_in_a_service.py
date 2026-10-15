"""A small web service that crawls one category of https://books.toscrape.com/ when called.

    GET /books?category=<name>   ->   [{"title": "...", "price": "..."}, ...]

How it is built:

* Each call opens its own Foxhound connection and its own crawl Run (a generated Run ID), so two calls running at the
  same time in one process share nothing but the declarations below, which hold no per-call state: the category
  travels in the seed's Payload. Calling again later works the same, since nothing is left behind.
* If the caller disconnects, the crawl task is cancelled; whatever it opened (the Run, its pages and browsers, the
  connection) is closed in a shielded cleanup that still runs under cancellation.
* The library's logs (the `goodfetch` loggers) are forwarded to the service's own logger. Nothing is printed to
  standard output.

Notebook cell (Python), using the sync API, which works on a thread whose event loop is already running::

    from scenarios.s12_crawl_in_a_service import crawl_books_blocking

    crawl_books_blocking("Travel")

A cell may also simply ``await crawl_books("Travel")``, the service's own async function.
"""

from __future__ import annotations

import logging
import re
import sys
from collections.abc import Awaitable, Callable
from datetime import timedelta
from typing import Annotated, TypeVar

import anyio
from fastapi import FastAPI, HTTPException, Query, Request
from pydantic import BaseModel, ConfigDict

from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncCrawlContext,
    AsyncFoxhound,
    Crawl,
    CrawlContext,
    CrawlRunReport,
    CrawlRunSettings,
    Foxhound,
    seed,
)

SITE = "https://books.toscrape.com/"
SERVICE_LOGGER_NAME = "books_service"
CLEANUP_TIMEOUT_SECONDS = 30.0
RUN_TIMEOUT = timedelta(minutes=5)
RUN_SETTINGS: CrawlRunSettings = {"crawl": {"max_requests": 500}}

log = logging.getLogger(SERVICE_LOGGER_NAME)

_T = TypeVar("_T")


# --------------------------------------------------------------------------------------------------------------------
# Logging: the library logs under `goodfetch`; forward them to the service's own logger. Standard error, never stdout.
# --------------------------------------------------------------------------------------------------------------------


class _ForwardToService(logging.Handler):
    """Hands every record of the library's loggers to the service's logger, which decides what to do with it."""

    def emit(self, record: logging.LogRecord) -> None:
        log.handle(record)


def configure_logging() -> None:
    """Idempotent. The service logger gets a stderr handler unless the host configured one."""
    if not log.handlers and not logging.getLogger().handlers:
        handler = logging.StreamHandler(sys.stderr)
        handler.setFormatter(logging.Formatter("%(asctime)s %(levelname)s %(name)s: %(message)s"))
        log.addHandler(handler)
        log.setLevel(logging.INFO)
    library = logging.getLogger("goodfetch")
    if not any(isinstance(h, _ForwardToService) for h in library.handlers):
        library.addHandler(_ForwardToService())
    library.propagate = False
    library.setLevel(logging.DEBUG)


# --------------------------------------------------------------------------------------------------------------------
# The data
# --------------------------------------------------------------------------------------------------------------------


class Book(BaseModel):
    title: str
    price: str


class Home(BaseModel):
    """The payload of the first request: which category to look for in the site's sidebar."""

    model_config = ConfigDict(frozen=True)

    category: str


class CategoryNotFoundError(Exception):
    """The site has no category of that name."""


class CrawlIncompleteError(Exception):
    """The crawl ended without covering its scope, so its books would be partial."""


def category_slug(name: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", name.strip().lower()).strip("-")


def category_pattern(name: str) -> re.Pattern[str]:
    """The sidebar link of a category: `.../catalogue/category/books/<slug>_<id>/index.html`."""
    slug = re.escape(category_slug(name))
    return re.compile(rf"^https://books\.toscrape\.com/catalogue/category/books/{slug}_\d+/index\.html$")


def check_report(report: CrawlRunReport) -> None:
    if report.requests.skipped:
        raise CategoryNotFoundError
    if report.lifecycle != "finished" or report.completeness != "complete" or report.requests.failed or report.requests.blocked:
        raise CrawlIncompleteError(
            f"run {report.id}: {report.lifecycle}, {report.completeness}, "
            f"{report.requests.failed} failed, {report.requests.blocked} blocked"
        )


# --------------------------------------------------------------------------------------------------------------------
# The crawl, async (the service)
# --------------------------------------------------------------------------------------------------------------------

CRAWL = AsyncCrawl()
HOME = CRAWL.label("home", payload=Home)
LISTING = CRAWL.label("listing")
BOOKS = CRAWL.dataset("books", schema=Book)


@CRAWL.handle(HOME)
async def handle_home(ctx: AsyncCrawlContext[Home]) -> None:
    links = ctx.page.locator(".side_categories a")
    found = await ctx.enqueue_links(links, LISTING, include=[category_pattern(ctx.request.payload.category)])
    if not found.added:
        ctx.skip(f"unknown category {ctx.request.payload.category!r}")


@CRAWL.handle(LISTING)
async def handle_listing(ctx: AsyncCrawlContext[None]) -> None:
    for row in await ctx.page.locator("article.product_pod").all():
        link = row.locator("h3 a")
        title = await link.get_attribute("title")
        if title is None:
            title = await link.text_content()
        price = await row.locator("p.price_color").text_content()
        await ctx.push(BOOKS, Book(title=title.strip(), price=price.strip()))
    await ctx.enqueue_links(ctx.page.locator("li.next a"), LISTING)


async def crawl_books(category: str) -> list[Book]:
    """Crawls one category with its own connection and Run; releases both on every path, cancellation included."""
    fox = await AsyncFoxhound.connect()
    try:
        run = await fox.start_crawl(
            CRAWL,
            seeds=[seed(SITE + "index.html", HOME, Home(category=category))],
            settings=RUN_SETTINGS,
            timeout=RUN_TIMEOUT,
        )
        try:
            report = await run.finish()
            check_report(report)
            books: list[Book] = []
            async with run.records(BOOKS) as records:
                async for record in records:
                    books.append(record.value)
            log.info("category %r: %d books in run %s", category, len(books), run.id)
            return books
        finally:
            with anyio.move_on_after(CLEANUP_TIMEOUT_SECONDS, shield=True):
                await run.close()
    finally:
        with anyio.move_on_after(CLEANUP_TIMEOUT_SECONDS, shield=True):
            await fox.close()


# --------------------------------------------------------------------------------------------------------------------
# The same crawl, sync (a notebook cell: `Foxhound` runs its async core on its own thread)
# --------------------------------------------------------------------------------------------------------------------

SYNC_CRAWL = Crawl()
SYNC_HOME = SYNC_CRAWL.label("home", payload=Home)
SYNC_LISTING = SYNC_CRAWL.label("listing")
SYNC_BOOKS = SYNC_CRAWL.dataset("books", schema=Book)


@SYNC_CRAWL.handle(SYNC_HOME)
def handle_home_sync(ctx: CrawlContext[Home]) -> None:
    links = ctx.page.locator(".side_categories a")
    found = ctx.enqueue_links(links, SYNC_LISTING, include=[category_pattern(ctx.request.payload.category)])
    if not found.added:
        ctx.skip(f"unknown category {ctx.request.payload.category!r}")


@SYNC_CRAWL.handle(SYNC_LISTING)
def handle_listing_sync(ctx: CrawlContext[None]) -> None:
    for row in ctx.page.locator("article.product_pod").all():
        link = row.locator("h3 a")
        title = link.get_attribute("title")
        if title is None:
            title = link.text_content()
        price = row.locator("p.price_color").text_content()
        ctx.push(SYNC_BOOKS, Book(title=title.strip(), price=price.strip()))
    ctx.enqueue_links(ctx.page.locator("li.next a"), SYNC_LISTING)


def crawl_books_blocking(category: str) -> list[Book]:
    """The same crawl for a notebook cell or a plain script; the `with` blocks release the Run and the connection."""
    with Foxhound.connect() as fox:
        with fox.start_crawl(
            SYNC_CRAWL,
            seeds=[seed(SITE + "index.html", SYNC_HOME, Home(category=category))],
            settings=RUN_SETTINGS,
            timeout=RUN_TIMEOUT,
        ) as run:
            report = run.finish()
            check_report(report)
            with run.records(SYNC_BOOKS) as records:
                return [record.value for record in records]


# --------------------------------------------------------------------------------------------------------------------
# The service
# --------------------------------------------------------------------------------------------------------------------


async def run_until_disconnect(request: Request, work: Callable[[], Awaitable[_T]]) -> _T:
    """Runs `work`; if the caller disconnects first, cancels it (its own cleanup is shielded) and raises 499."""
    values: list[_T] = []
    errors: list[Exception] = []
    async with anyio.create_task_group() as group:

        async def do_work() -> None:
            try:
                values.append(await work())
            except Exception as exc:  # carried out of the task group, so it is not wrapped in an ExceptionGroup
                errors.append(exc)
            group.cancel_scope.cancel()

        async def watch() -> None:
            while not await request.is_disconnected():
                await anyio.sleep(0.2)
            group.cancel_scope.cancel()

        group.start_soon(do_work)
        group.start_soon(watch)
    if errors:
        raise errors[0]
    if not values:
        raise HTTPException(status_code=499, detail="client disconnected")
    return values[0]


configure_logging()
app = FastAPI(title="Books service")


@app.get("/books")
async def books(request: Request, category: Annotated[str, Query(min_length=1)]) -> list[Book]:
    try:
        return await run_until_disconnect(request, lambda: crawl_books(category))
    except CategoryNotFoundError:
        raise HTTPException(status_code=404, detail=f"unknown category {category!r}") from None
    except CrawlIncompleteError as exc:
        log.warning("incomplete crawl for %r: %s", category, exc)
        raise HTTPException(status_code=502, detail="the crawl did not complete") from None
