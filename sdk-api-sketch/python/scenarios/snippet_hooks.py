"""Snippet for Hooks and injectable components in the SDK API (#481, ADR 0104), on golden scenario 05's crawl."""

import logging

from pydantic import BaseModel

from goodfetch import (
    AsyncAfterResponseContext,
    AsyncBeforeRequestContext,
    AsyncCrawl,
    AsyncCrawlContext,
    AsyncGoodfetch,
    AsyncPageStageContext,
    AsyncRecordCommitContext,
    ProxyAnswer,
    ProxyRequest,
    RequestChange,
    secret,
    seed,
)

log = logging.getLogger(__name__)


async def lease_proxy(domain: str) -> tuple[str, str]:  # the program's own proxy pool
    raise NotImplementedError


class Book(BaseModel):
    title: str
    price: str
    url: str


class BookPayload(BaseModel):
    category: str


crawl = AsyncCrawl()
books = crawl.dataset("books", schema=Book, key=("url",))
home = crawl.label("home")
book = crawl.label("book", payload=BookPayload)


# Before request, scoped by Label handle: the Payload is typed, and the change is a returned value. The header's value
# is a Secret placeholder, which the engine adds after the last Hook, so the key never passes through the program.
@crawl.before_request(labels=[book], name="catalog-key")
async def catalog_key(ctx: AsyncBeforeRequestContext[BookPayload]) -> RequestChange:
    # Also the stage before a retry: stop retrying a request the site keeps timing out on.
    if ctx.previous_error is not None and ctx.previous_error.code == "deadline_exceeded" and ctx.request.attempt > 2:
        ctx.skip(f"gave up on {ctx.request.payload.category}")
    return RequestChange(headers={"x-catalog-key": secret("catalog_key"), "x-category": ctx.request.payload.category})


# After response: a soft block served with a 200 is marked here, and feeds escalation by new Session.
@crawl.after_response()
async def soft_block(ctx: AsyncAfterResponseContext[object]) -> None:
    if ctx.response.status == 200 and "Access denied" in await ctx.response.text():
        ctx.block("ip_blocked", "soft block page")


# Page ready, unscoped: runs just before every handler, on every tier, as Crawlee's `router.use`.
@crawl.page_ready()
async def trace(ctx: AsyncPageStageContext[object]) -> None:
    log.debug("handling %s (%s) on %s", ctx.request.url, ctx.request.label.name, ctx.tier)


# Before a Record's commit, scoped by Dataset: it sees and returns the Dataset's Record type.
@crawl.before_record_commit(datasets=[books], name="trim-title")
async def trim_title(ctx: AsyncRecordCommitContext[object, Book]) -> Book:
    if not ctx.record.title.strip():
        ctx.drop("no title")
    return ctx.record.model_copy(update={"title": ctx.record.title.strip()})


@crawl.handle(home)
async def handle_home(ctx: AsyncCrawlContext[None]) -> None:
    await ctx.enqueue_links(ctx.page.locator("article.product_pod h3 a"), book, BookPayload(category="all"))


@crawl.handle(book)
async def handle_book(ctx: AsyncCrawlContext[BookPayload]) -> None:
    title = await ctx.page.locator("h1").text_content()
    await ctx.push(books, Book(title=title, price=await ctx.page.locator(".price_color").text_content(), url=ctx.page.url))


# The program's own Proxy provider, an SDK handler registered on the connection under its name, as ADR 0101 registers
# `model_providers`: the only one of its kind there, so it is the connection's choice for proxies.
async def office_pool(request: ProxyRequest) -> ProxyAnswer:
    host, user = await lease_proxy(request.domain)
    return ProxyAnswer(server=f"http://{host}:3128", username=user, password="caller:office-proxy")


async def crawl_with_hooks() -> None:
    async with AsyncGoodfetch.connect(
        proxy_providers={"office-pool": office_pool},
        settings={"secrets": {"catalog_key": {"reference": "env:CATALOG_KEY", "domains": ["books.toscrape.com"]}}},
    ) as gf:
        run = await gf.foxhound.start_crawl(crawl, seeds=[seed("https://books.toscrape.com/", home)])
        async with run:
            pass


# Typed mistakes, caught before anything runs: a Hook scoped to `books` returns a Book, and a crawl request stays a GET.
@crawl.before_record_commit(datasets=[books])  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]
async def wrong_record(ctx: AsyncRecordCommitContext[object, Book]) -> str:
    return "not a Book"


def no_method() -> RequestChange:
    return RequestChange(method="POST")  # type: ignore[call-arg]  # pyright: ignore[reportCallIssue]
