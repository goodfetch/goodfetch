"""Snippet for Capping a crawl's model use without stopping its Run (#513, ADR 0107), on golden scenario 08."""

import logging
from collections.abc import Sequence
from decimal import Decimal
from typing import Literal

from pydantic import BaseModel, Field

from goodfetch import (
    AsyncCrawl,
    AsyncCrawlContext,
    AsyncGoodfetch,
    AsyncPage,
    CrawlRequestSkipped,
    ModelAllowanceReachedError,
    Money,
    Stopped,
    seed,
)

log = logging.getLogger(__name__)


class Book(BaseModel):
    title: str
    price: Decimal = Field(decimal_places=2)
    in_stock: int
    rating: Literal[1, 2, 3, 4, 5]


class BookRecord(Book):
    url: str


# Scenario 08: the model is asked once per page type (the Plan), and stops being asked once the crawl has spent
# 0.50 USD. The crawl goes on: books whose layout the Plan learned are still extracted with no model call, and a book
# that needs the model is skipped with the reason `model_allowance`.
crawl = AsyncCrawl()
books = crawl.dataset("books", schema=BookRecord, key=("url",))
category = crawl.label("category")
book = crawl.label("book")


@crawl.handle(category)
async def on_category(ctx: AsyncCrawlContext[None]) -> None:
    await ctx.enqueue_links(ctx.page.locator("article.product_pod h3 a"), book)
    await ctx.enqueue_links(ctx.page.locator("li.next a"), category)


@crawl.handle(book)
async def on_book(ctx: AsyncCrawlContext[None]) -> None:
    # Past the allowance, a step with no Plan entry or a Repair raises ModelAllowanceReachedError: left to escape, it
    # ends this request `skipped` with the reason `model_allowance`, not `failed`.
    fields = (await ctx.page.extract("the book on this page", Book)).output
    await ctx.push(books, BookRecord(**fields.model_dump(), url=ctx.page.url))


async def scenario_08(categories: Sequence[str]) -> None:
    async with AsyncGoodfetch.connect(settings={"ai": {"model": "vercel/openai/gpt-5-mini"}}) as gf:
        run = await gf.foxhound.start_crawl(
            crawl,
            seeds=[seed(url, category) for url in categories],
            plan="books.plan.json",  # learned once per page type, replayed with no model call (ADR 0091)
            # The Model allowance: not a Limit, so the crawl goes on once it is reached.
            settings={"ai": {"allowance": {"price": Money(amount=Decimal("0.50"), currency="USD")}}},
        )
        async with run:
            async for outcome in run.outcomes(kinds=["skipped"]):
                match outcome:
                    case CrawlRequestSkipped(request=request, reason="model_allowance", message=message):
                        log.info("left unextracted: %s (%s)", request.url, message)
                    case _:
                        pass
            report = await run.finish()  # `finished`, and `incomplete` when any book was left
            reached = [a.reached_at for a in report.usage.model_allowance]
            log.info(
                "model use: %s tokens, %s; allowance reached at %s; %s %s",
                report.usage.model.tokens,
                report.usage.model.price,  # `None` only if a model had no price
                reached,
                report.completeness,
                report.completeness_reasons,
            )


# A handler that keeps a Record for every book catches the refusal: a skipped attempt keeps nothing.
class KeptBook(BaseModel):
    url: str
    book: Book | None
    why: str | None


keep_all = AsyncCrawl()
kept = keep_all.dataset("books", schema=KeptBook, key=("url",))
any_book = keep_all.label("book")


@keep_all.handle(any_book)
async def on_any_book(ctx: AsyncCrawlContext[None]) -> None:
    try:
        fields = (await ctx.page.extract("the book on this page", Book)).output
        await ctx.push(kept, KeptBook(url=ctx.page.url, book=fields, why=None))
    except ModelAllowanceReachedError:
        await ctx.push(kept, KeptBook(url=ctx.page.url, book=None, why="the Model allowance was reached"))


# An Agent's task ends as a value when the allowance is reached, and the Run goes on.
async def agent_stops(page: AsyncPage) -> Stopped | None:
    result = await page.agent("add the cheapest poetry book to the basket").result()
    if isinstance(result.ended, Stopped) and result.ended.reason == "model_allowance":
        log.warning("the Model allowance stopped the task: %s", result.ended.source)
        return result.ended
    return None


# Counted per Run: a Session takes none.
async def not_per_session() -> None:
    async with AsyncGoodfetch.connect() as gf, await gf.collie.start(settings={"ai": {"allowance": {"calls": 50}}}) as run:
        await run.new_session(settings={"ai": {"allowance": {"calls": 5}}})  # type: ignore[typeddict-unknown-key]  # pyright: ignore[reportArgumentType]
