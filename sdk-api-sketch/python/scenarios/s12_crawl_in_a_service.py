"""Golden scenario 12, "Run a crawl inside a web service" (Foxhound, Python).

Run the service with `uvicorn scenarios.s12_crawl_in_a_service:app`; `notebook_cell` is the same crawl from a notebook.
"""

import logging
from collections.abc import AsyncGenerator
from contextlib import asynccontextmanager

import anyio
from fastapi import FastAPI, HTTPException, Request
from pydantic import BaseModel, Field

from goodfetch_foxhound import AsyncCrawl, AsyncCrawlContext, AsyncFoxhound, seed

# Foxhound logs to `logging` loggers under `goodfetch` and adds only a `NullHandler`: the service's own logging
# configuration (uvicorn's here) decides where they go, and nothing reaches standard output.
log = logging.getLogger("bookshop")


class Book(BaseModel):
    title: str = Field(min_length=1)
    price: str


class Category(BaseModel):
    name: str


# The declaration holds no Run's state, so every call, at once or later, shares it; importing it does nothing.
crawl = AsyncCrawl()
books = crawl.dataset("books", schema=Book)
home = crawl.label("home", payload=Category)
category_page = crawl.label("category")


@crawl.handle(home)
async def find_category(ctx: AsyncCrawlContext[Category]) -> None:
    name = ctx.request.payload.name
    link = ctx.page.locator(".side_categories ul ul").get_by_role(
        "link", name=name, exact=True
    )
    if not (await ctx.enqueue_links(link, category_page)).added:
        ctx.skip(f"no category named {name!r}")


@crawl.handle(category_page)
async def read_books(ctx: AsyncCrawlContext[None]) -> None:
    for book in await ctx.page.locator("article.product_pod").all():
        # A missing title attribute fails `min_length`: the request fails with the ValidationError.
        title = await book.locator("h3 a").get_attribute("title") or ""
        await ctx.push(
            books,
            Book(title=title, price=await book.locator(".price_color").text_content()),
        )
    await ctx.enqueue_links(ctx.page.locator("li.next a"), ctx.request.label)


async def books_in(foxhound: AsyncFoxhound, name: str) -> list[Book] | None:
    """The category's books, or None when the site has no such category. Each call is a Run of its own, so calls
    running at once share no state. Cancelling the task ends `finish()`'s wait, and leaving the block then closes the
    Run: closing an unfinished Run stops it with a Checkpoint and closes its Sessions, releasing its browser and
    connections (ADRs 0078, 0102)."""
    seeds = [seed("https://books.toscrape.com/", home, Category(name=name))]
    async with await foxhound.start_crawl(crawl, seeds=seeds) as run:
        report = await run.finish()
        if report.requests.by_skip_reason.get("handler"):
            return None
        return [record.value async for record in run.records(books)]


@asynccontextmanager
async def lifespan(app: FastAPI) -> AsyncGenerator[None, None]:
    # One connection for the service's life: Foxhound installs no signal handler, so uvicorn's own stops it.
    async with AsyncFoxhound.connect() as foxhound:
        app.state.foxhound = foxhound
        yield


app = FastAPI(lifespan=lifespan)


async def cancel_when_gone(request: Request, scope: anyio.CancelScope) -> None:
    """Starlette does not cancel an endpoint whose caller disconnects: this watches for it."""
    while (await request.receive())["type"] != "http.disconnect":
        pass
    scope.cancel()


@app.get("/books")
async def get_books(category: str, request: Request) -> list[Book]:
    found: list[Book] | None = []  # what a disconnected caller gets: nobody reads it
    async with anyio.create_task_group() as tasks:
        tasks.start_soon(cancel_when_gone, request, tasks.cancel_scope)
        found = await books_in(request.app.state.foxhound, category)
        tasks.cancel_scope.cancel()  # the crawl ended: stop watching
    if found is None:
        raise HTTPException(status_code=404, detail=f"no category named {category!r}")
    log.info("crawled %s: %d books", category, len(found))
    return found


async def notebook_cell() -> list[Book] | None:
    """The same crawl from a notebook cell, written as the cell's body. IPython runs a cell's top-level `await` on its
    running loop, under which the async client works (ADR 0078); nothing blocks that loop."""
    async with AsyncFoxhound.connect() as foxhound:
        return await books_in(foxhound, "Travel")
