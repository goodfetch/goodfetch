"""Snippet for Crawl output in code (#479, ADR 0103), on golden scenarios 05, 07 and 12."""

import logging
import re
from collections.abc import Awaitable, Callable
from decimal import Decimal
from urllib.parse import urljoin

from pydantic import BaseModel, Field

from goodfetch import AsyncCrawl, AsyncCrawlContext, AsyncCrawlRun, AsyncGoodfetch, RecordInvalidError, seed

log = logging.getLogger(__name__)


class Book(BaseModel):
    title: str
    price: Decimal = Field(decimal_places=2)
    url: str
    category: str


class Cursor(BaseModel):
    last_page: int


class CategoryPayload(BaseModel):
    name: str


# Datasets are declared on the crawl, as its Labels are: no connection, no I/O, no default Dataset. The key identifies a
# book collected twice (two categories, or tomorrow's run): Sinks upsert on it.
crawl = AsyncCrawl()
books = crawl.dataset("books", schema=Book, key=("url",))
skipped = crawl.dataset("skipped")  # no schema: any JSON object
category = crawl.label("category", payload=CategoryPayload)


@crawl.handle(category)
async def on_category(ctx: AsyncCrawlContext[CategoryPayload]) -> None:
    for pod in await ctx.page.locator("article.product_pod").all():
        price = re.search(r"£(\d+\.\d{2})", await pod.locator(".price_color").text_content())
        url = urljoin(ctx.page.url, await pod.locator("h3 a").get_attribute("href") or "")
        if price is None:
            await ctx.push(skipped, {"url": url, "why": "no price"})
            continue
        # Staged with this attempt: it commits with it, numbered, or is discarded if the attempt fails, retries or
        # escalates, so a retried page never doubles the Dataset (P18).
        title = await pod.locator("h3 a").get_attribute("title") or ""
        await ctx.push(books, Book(title=title, price=Decimal(price[1]), url=url, category=ctx.request.payload.name))
    # Staged too, through the attempt's `run`: the cursor advances only if this page's Records commit.
    await ctx.run.key_value_store.set(f"cursor/{ctx.request.payload.name}", Cursor(last_page=ctx.request.depth))
    await ctx.enqueue_links(ctx.page.locator("li.next a"), ctx.request.label, ctx.request.payload)


async def follow(run: AsyncCrawlRun, last_handled: int, save: Callable[[int], Awaitable[None]]) -> None:
    """Scenario 12: a crawl in a service reads its Records as they commit, and resumes after a restart from the last
    number it handled. The reader is a cursor over run state: slow or paused, it never slows the crawl."""
    try:
        async for record in run.records(books, after=last_handled):
            log.info("book %d: %s from %s", record.number, record.value.title, record.url)
            await save(record.number)
    except RecordInvalidError as error:
        # A Record the model's own validators now reject ends the loop with its number.
        log.error("unreadable record %s", error.details.number)
        raise
    async for rejected in run.rejected_records(books):
        log.warning("rejected by %s: %s", rejected.hook, rejected.reason)


async def scenario_05() -> None:
    async with AsyncGoodfetch.connect() as gf:
        async with await gf.foxhound.start(
            crawl,
            seeds=[seed("https://books.toscrape.com/catalogue/category/books/travel_2/index.html", category, CategoryPayload(name="Travel"))],
            # A Sink is a setting: from code, the project file, the CLI or a Recipe alike.
            settings={"sinks": {"warehouse": {"type": "postgres", "datasets": ["books"], "mode": "upsert"}}},
        ) as run:
            cursor = await run.key_value_store.get("cursor/Travel", Cursor)  # typed by its model, or None
            log.info("last page %s", cursor.last_page if cursor else None)
            # An Export of what has committed so far, saved on this machine wherever the engine runs.
            csv = await run.export(books, format="csv")
            await csv.save_as("books.csv")
            log.info("exported %d records up to %d (%s)", csv.records, csv.last_number, csv.sha256)


async def mistakes(ctx: AsyncCrawlContext[None], run: AsyncCrawlRun) -> None:
    """Typed mistakes, caught before anything runs."""
    # A plain dict where the Dataset takes its model is a type error.
    await ctx.push(books, {"title": "x", "price": "1.00", "url": "u", "category": "c"})  # type: ignore[misc]  # pyright: ignore[reportArgumentType]
    # The format is named, never guessed from a file name.
    await run.export(books, format="cvs")  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]
