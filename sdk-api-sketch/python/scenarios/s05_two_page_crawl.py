"""Golden scenario 05: crawl two page types into a Dataset. Scenarios 07 and 10 import its crawl; it runs only as a
script (`python -m scenarios.s05_two_page_crawl`)."""

import asyncio
import re
from decimal import Decimal

from pydantic import AnyUrl, BaseModel, Field

from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncCrawlContext,
    AsyncFoxhound,
    CrawlRequestBlocked,
    CrawlRequestFailed,
    CrawlRequestSkipped,
    seed,
)


class Book(BaseModel):
    title: str = Field(min_length=1)
    price: Decimal = Field(ge=0, decimal_places=2)  # pounds, exact
    in_stock: int = Field(ge=0)
    rating: int = Field(ge=1, le=5)
    url: AnyUrl
    category: str = Field(min_length=1)


class CategoryPayload(BaseModel):
    name: str = Field(min_length=1)


class BookPayload(BaseModel):
    category: str = Field(min_length=1)


# The declaration does no I/O, so the module holds it and importing it runs nothing.
crawl = AsyncCrawl()
books = crawl.dataset("books", schema=Book, key=("url",))
home = crawl.label("home")
category = crawl.label("category", payload=CategoryPayload)
book = crawl.label("book", payload=BookPayload)
# Staying on books.toscrape.com is the default scope (the seed's host), and robots.txt, one request at a time with a
# second between starts and backing off on 429 are the default Politeness.
seeds = [seed("https://books.toscrape.com/", home)]

RATINGS = ("One", "Two", "Three", "Four", "Five")


@crawl.handle(home)
async def on_home(ctx: AsyncCrawlContext[None]) -> None:
    # The nested list leaves out the top-level "Books"; each link's own text is its Payload.
    links = ctx.page.locator(".side_categories ul ul a")
    await ctx.enqueue_links(
        links, category, lambda link: CategoryPayload(name=link.text.strip())
    )


@crawl.handle(category)
async def on_category(ctx: AsyncCrawlContext[CategoryPayload]) -> None:
    name = ctx.request.payload.name
    await ctx.enqueue_links(
        ctx.page.locator("article.product_pod h3 a"), book, BookPayload(category=name)
    )
    await ctx.enqueue_links(
        ctx.page.locator("li.next a"), ctx.request.label, ctx.request.payload
    )


@crawl.handle(book)
async def on_book(ctx: AsyncCrawlContext[BookPayload]) -> None:
    main = ctx.page.locator(".product_main")
    text = await main.text_content()
    price = re.search(r"£(\d+\.\d{2})", text)
    stock = re.search(r"\((\d+) available\)", text)
    stars = await main.locator(".star-rating").get_attribute("class") or ""
    rating = next((n for n, word in enumerate(RATINGS, 1) if word in stars), None)
    if price is None or stock is None or rating is None:
        ctx.skip(f"no price, stock or rating on the page (stars {stars!r})")
    await ctx.push(
        books,
        Book(
            title=await main.locator("h1").text_content(),
            price=Decimal(price[1]),
            in_stock=int(stock[1]),
            rating=rating,
            url=AnyUrl(ctx.page.url),  # the final URL, after redirects
            category=ctx.request.payload.category,
        ),
    )


async def main() -> None:
    async with (
        AsyncFoxhound.connect() as foxhound,
        await foxhound.start_crawl(crawl, seeds=seeds) as run,
    ):
        report = await run.finish()
        exported = await run.export(books, format="json")
        await exported.save_as("books.json")
        print(f"exported {exported.records} books to books.json")

        async for outcome in run.outcomes(kinds=["failed", "skipped", "blocked"]):
            match outcome:
                case CrawlRequestFailed(request=request, error=error):
                    print(f"failed {request.url}: {error.code}: {error}")
                case CrawlRequestSkipped(
                    request=request, reason=reason, message=message
                ):
                    print(f"skipped {request.url}: {reason} {message or ''}")
                case CrawlRequestBlocked(request=request, block_cause=cause):
                    print(f"blocked {request.url}: {cause}")
                case _:
                    pass
        totals = report.requests
        print(
            f"{totals.succeeded} succeeded, {totals.failed} failed, {totals.skipped} skipped, {totals.blocked} blocked"
        )
        if report.nothing_done is not None:
            print(f"nothing done: {report.nothing_done.reason}")


if __name__ == "__main__":
    asyncio.run(main())
