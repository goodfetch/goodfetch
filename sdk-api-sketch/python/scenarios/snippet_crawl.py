"""Snippet for Crawl definition and handler context (#477, ADR 0093), on golden scenarios 05, 06 and 08."""

import logging
import re
from collections import Counter
from collections.abc import AsyncIterable
from decimal import Decimal
from typing import Literal, TypeVar
from urllib.parse import urljoin, urlsplit

from pydantic import BaseModel, Field

from goodfetch import (
    AsyncCrawl,
    AsyncCrawlContext,
    AsyncGoodfetch,
    AsyncPage,
    AsyncRoute,
    CrawlRequestFailed,
    CrawlRequestOutcome,
    CrawlRequestSkipped,
    Dataset,
    seed,
)

log = logging.getLogger(__name__)
T = TypeVar("T")

# Provisional, each its owner's to decide: Datasets and push are #479's, reading outcomes #478's. `page.extract` is ADR
# 0091's.


def dataset(name: str, schema: type[T]) -> Dataset[T]:
    raise NotImplementedError


def outcomes_of(run_id: str) -> AsyncIterable[CrawlRequestOutcome]:
    raise NotImplementedError


# Scenario 05: three kinds of page, the category name passed as a typed Payload.
class Book(BaseModel):
    title: str
    price: Decimal = Field(decimal_places=2)
    in_stock: int
    rating: Literal[1, 2, 3, 4, 5]
    url: str
    category: str


class CategoryPayload(BaseModel):
    name: str = Field(min_length=1)


class BookPayload(BaseModel):
    category: str = Field(min_length=1)


books = dataset("books", Book)

# The declaration needs no connection: the module holds it, and importing it does nothing.
crawl = AsyncCrawl()
home = crawl.label("home")
category = crawl.label("category", payload=CategoryPayload)
book = crawl.label("book", payload=BookPayload)


@crawl.handle(home)
async def on_home(ctx: AsyncCrawlContext[None]) -> None:
    # Every category but the top-level "Books": the name read here travels with each request.
    for link in await ctx.page.locator(".side_categories ul ul a").all():
        href = await link.get_attribute("href")
        if href is not None:
            name = (await link.text_content()).strip()
            await ctx.enqueue(urljoin(ctx.page.url, href), category, CategoryPayload(name=name))


@crawl.handle(category)
async def on_category(ctx: AsyncCrawlContext[CategoryPayload]) -> None:
    name = ctx.request.payload.name
    added = await ctx.enqueue_links(ctx.page.locator("article.product_pod h3 a"), book, BookPayload(category=name))
    if not added.added:
        log.warning("no book links on %s: %s", ctx.page.url, added.not_added)
    # Pagination: the request's own Label.
    await ctx.enqueue_links(ctx.page.locator("li.next a"), ctx.request.label, ctx.request.payload)


@crawl.handle(book)
async def on_book(ctx: AsyncCrawlContext[BookPayload]) -> None:
    main = ctx.page.locator(".product_main")
    price = await main.locator(".price_color").text_content()  # waits; over HTTP, a missing price escalates
    match = re.search(r"£(\d+\.\d{2})", price)
    if match is None:
        ctx.skip(f"no price in {price!r}")  # ends the request `skipped`, reason `handler`
    stock = re.search(r"(\d+) available", await main.locator(".availability").text_content())
    stars = await main.locator(".star-rating").get_attribute("class") or ""
    rating = next((i for i, w in enumerate(["One", "Two", "Three", "Four", "Five"], 1) if w in stars), 1)
    await books.push(
        Book.model_validate(
            {
                "title": await main.locator("h1").text_content(),
                "price": Decimal(match[1]),
                "in_stock": int(stock[1]) if stock else 0,
                "rating": rating,
                "url": ctx.page.url,  # the final URL, after redirects
                "category": ctx.request.payload.category,
            }
        )
    )


async def scenario_05() -> Counter[str]:
    async with AsyncGoodfetch.connect() as gf:
        run = await gf.foxhound.start(
            crawl,
            seeds=[seed("https://books.toscrape.com/", home)],
            settings={"crawl": {"scope": {"strategy": "same_hostname"}}},  # the built-in, written out
        )
        async with run:
            counts: Counter[str] = Counter()
            async for outcome in outcomes_of(run.id):
                counts[type(outcome).__name__] += 1
                match outcome:
                    case CrawlRequestFailed(request=request, error=error):
                        log.warning("failed %s: %s (%s)", request.url, error.code, error.fix)
                    case CrawlRequestSkipped(request=request, reason=reason, message=message):
                        log.info("skipped %s: %s %s", request.url, reason, message or "")
                    case _:
                        pass
            return counts


# Typed mistakes, caught by the type checkers before anything runs.
async def mistakes(ctx: AsyncCrawlContext[CategoryPayload]) -> None:
    await ctx.enqueue("https://books.toscrape.com/x", book, CategoryPayload(name="Travel"))  # type: ignore[misc]  # pyright: ignore[reportCallIssue, reportArgumentType]
    await ctx.enqueue("https://books.toscrape.com/x", book)  # type: ignore[arg-type]  # pyright: ignore[reportCallIssue, reportArgumentType]
    crawl.label("listing", settings={"politeness": {"domain": {"concurrency": 4}}})  # type: ignore[call-overload]  # pyright: ignore[reportArgumentType, reportCallIssue]
    await ctx.enqueue("https://books.toscrape.com/", home, CategoryPayload(name="x"))  # type: ignore[misc]  # pyright: ignore[reportCallIssue, reportArgumentType]
    crawl.label("localized", settings={"browser": {"locale": "fr-FR"}})  # type: ignore[call-overload]  # pyright: ignore[reportArgumentType, reportCallIssue]


# Scenario 06: one handler over HTTP, the JavaScript version and the scrolling one; no second crawler.
class Quote(BaseModel):
    text: str
    author: str
    tags: list[str]
    version: str


quotes = dataset("quotes", Quote)
quotes_crawl = AsyncCrawl()
quote_page = quotes_crawl.label("quotes")
# The scrolling page reads the JSON the page loads, which only a browser does: its own Label forces it.
scroll_page = quotes_crawl.label("scroll", settings={"fetch": {"tier": "browser"}})


@quotes_crawl.handle(quote_page)
async def on_quotes(ctx: AsyncCrawlContext[None]) -> None:
    # Waiting for a quote is what this page must hold: over HTTP, /js/'s empty document escalates the request to the
    # browser instead of passing as an empty success (failure point 1).
    await ctx.page.locator(".quote").first.wait_for()
    for q in await ctx.page.locator(".quote").all():
        await quotes.push(
            Quote(
                text=await q.locator(".text").text_content(),
                author=await q.locator(".author").text_content(),
                tags=await q.locator(".tag").all_text_contents(),
                version="js" if urlsplit(ctx.request.url).path.startswith("/js") else "html",
            )
        )
    log.debug("fetched %s over %s", ctx.request.url, ctx.tier)  # `http` for /; `browser` for /js/ once learned, if area 6 splits the Label by path
    await ctx.enqueue_links(ctx.page.locator("li.next a"), ctx.request.label)


async def block_media(route: AsyncRoute) -> None:
    await route.abort("blockedbyclient")


@quotes_crawl.handle(scroll_page)
async def on_scroll(ctx: AsyncCrawlContext[None]) -> None:
    with ctx.page.route("**/*.{png,jpg,woff,woff2}", block_media):
        async with ctx.page.expect_request_finished(re.compile(r"/api/quotes\?page=\d+")) as last:
            await ctx.page.mouse.wheel(0, 10_000)
        response = await (await last.value).response()
        if response is not None and response.ok:
            log.info("scrolled: %s", await response.json())


# Scenario 08: an AI extraction on the crawl's own page, counted in the crawl's Run, learned once per Label on a domain.
class BookFields(BaseModel):
    title: str
    price: Decimal = Field(decimal_places=2)
    in_stock: int
    rating: Literal[1, 2, 3, 4, 5]


ai_crawl = AsyncCrawl()
ai_book = ai_crawl.label("book")


@ai_crawl.handle(ai_book)
async def on_ai_book(ctx: AsyncCrawlContext[None]) -> None:
    fields = (await ctx.page.extract("the book on this page", BookFields)).output
    if not fields.title:
        ctx.retry("the title had not rendered")  # `retry_requested`, under crawl.retry.max
    await books.push(Book(**fields.model_dump(), url=ctx.page.url, category="unknown"))


# A Label with Sessions of its own (ADRs 0092, 0093): the account's orders need the saved login, the catalogue does not.
async def sign_in(page: AsyncPage) -> None:
    await page.get_by_role("button", name="Sign in").click()


shop = AsyncCrawl()
catalogue = shop.label("catalogue")
orders = shop.label(
    "orders",
    settings={
        "session": {
            "source": {"kind": "saved_identity", "name": "shop-account"},
            "page_check": {"expected_page": {"selector": "table.orders"}, "login_page": {"url": "**/login*"}},
        }
    },
    login=sign_in,
)


@shop.handle(orders)
async def on_orders(ctx: AsyncCrawlContext[None]) -> None:
    log.info("orders: %d rows", await ctx.page.locator("table.orders tr").count())
