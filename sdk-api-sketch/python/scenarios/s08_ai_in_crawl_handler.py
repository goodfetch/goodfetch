"""Golden scenario 08: extract with AI inside a crawl, learning once per page type.

The declaration needs no connection, so scenario 10's offline tests import it; `main` runs only as a script.
"""

import asyncio
from datetime import timedelta
from decimal import Decimal
from typing import Literal

from pydantic import AnyUrl, BaseModel, Field

from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncCrawlContext,
    AsyncFoxhound,
    CrawlRequestFailed,
    CrawlRequestSkipped,
    Money,
    seed,
)


class Book(BaseModel):
    """What the model extracts from a book's page."""

    title: str = Field(min_length=1)
    price: Decimal = Field(ge=0, decimal_places=2)  # pounds, exact
    in_stock: int = Field(ge=0)
    rating: Literal[1, 2, 3, 4, 5]


class BookRecord(Book):
    url: AnyUrl


PLAN = "books.plan.json"
"""The Plan file: one entry for the book's extraction, compiled once per page layout (ADR 0091)."""

crawl = AsyncCrawl()
books = crawl.dataset("books", schema=BookRecord, key=("url",))
home = crawl.label("home")
category = crawl.label("category")
# A first compile runs inside its handler, so it gets more than `timeout.handler`'s 60 s.
book = crawl.label("book", settings={"timeout": {"handler": timedelta(minutes=3)}})


@crawl.handle(home)
async def on_home(ctx: AsyncCrawlContext[None]) -> None:
    await ctx.enqueue_links(
        ctx.page.locator(".side_categories ul ul a"), category, limit=3
    )


@crawl.handle(category)
async def on_category(ctx: AsyncCrawlContext[None]) -> None:
    await ctx.enqueue_links(ctx.page.locator("article.product_pod h3 a"), book)
    await ctx.enqueue_links(ctx.page.locator("li.next a"), category)


@crawl.handle(book)
async def on_book(ctx: AsyncCrawlContext[None]) -> None:
    # The first book compiles the Plan entry; the next replay it with no model call, and a page whose layout drifted
    # gets one Repair. Past the Model allowance, a step that needs the model raises ModelAllowanceReachedError:
    # escaping, it ends this request `skipped` (`model_allowance`).
    fields = (await ctx.page.extract("the book on this page", Book)).output
    await ctx.push(books, BookRecord(**fields.model_dump(), url=AnyUrl(ctx.page.url)))


async def main() -> None:
    # The Model allowance: not a Limit, so the crawl goes on once it is spent.
    allowance = Money(amount=Decimal("0.50"), currency="USD")
    model = "vercel/openai/gpt-5-mini"
    async with AsyncFoxhound.connect(settings={"ai": {"model": model}}) as foxhound:
        run = await foxhound.start_crawl(
            crawl,
            seeds=[seed("https://books.toscrape.com/", home)],
            plan=PLAN,
            settings={"ai": {"allowance": {"price": allowance}}},
        )
        async with run:
            left = run.outcomes(kinds=["failed", "skipped"], labels=[book])
            async for outcome in left:
                match outcome:
                    case CrawlRequestSkipped(request=request, reason=why, message=text):
                        print("unextracted", request.url, why, text)
                    case CrawlRequestFailed(request=request, error=error):
                        print("unextracted", request.url, error.code, error.fix)
                    case _:
                        pass
            # `incomplete` when a book was skipped for the allowance.
            report = await run.finish()
    model_use = report.usage.model
    print("model use:", model_use.tokens, "tokens,", model_use.price or "unpriced")


if __name__ == "__main__":
    asyncio.run(main())
