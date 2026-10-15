"""Golden scenario 06, "Crawl with HTTP and a browser only where needed" (Foxhound, Python)."""

import asyncio
import itertools
import re

from pydantic import BaseModel

from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncCrawlContext,
    AsyncFoxhound,
    AsyncPageStageContext,
    AsyncRoute,
    seed,
)


class Quote(BaseModel):
    text: str
    author: str
    tags: list[str]
    version: str


class Author(BaseModel):
    name: str


class ApiQuote(BaseModel):
    text: str
    author: Author
    tags: list[str]


class QuotesPage(BaseModel):
    """One page of the scrolling version's JSON API."""

    has_next: bool
    quotes: list[ApiQuote]


crawl = AsyncCrawl()
quotes = crawl.dataset("quotes", schema=Quote)
# The tier is learned per Label on a domain (`fetch.tier: auto`, the default), so each version that may need another
# tier is its own Label; the two share one handler.
html = crawl.label("html")
js = crawl.label("js")
# Scrolling and reading the JSON the page loads need a browser: this Label forces it, and keeps the API's responses from
# the start of its navigation, so page 1, loaded before the handler runs, is read without a reload.
scroll = crawl.label(
    "scroll",
    settings={
        "fetch": {
            "tier": "browser",
            "kept_responses": {"urls": [re.compile(r"/api/quotes\?page=\d+$")]},
        }
    },
)


def api_page(n: int) -> re.Pattern[str]:
    """The JSON page `n` the scrolling version loads."""
    return re.compile(rf"/api/quotes\?page={n}$")


async def drop_images_and_fonts(route: AsyncRoute) -> None:
    if route.request.resource_type in ("image", "font"):
        await route.abort("blockedbyclient")
    else:
        await route.fallback()


# Runs only where a browser navigates, so unscoped it covers every browser page, a sample comparison of an HTTP
# Label's page included.
@crawl.before_navigation(name="no-images-or-fonts")
async def block_images_and_fonts(ctx: AsyncPageStageContext[object]) -> None:
    ctx.page.route("**/*", drop_images_and_fonts)


@crawl.handle(js)
@crawl.handle(html)
async def read_quotes(ctx: AsyncCrawlContext[None]) -> None:
    # What the page must hold: over HTTP, /js/'s document has no quote, so this wait escalates the request to the
    # browser; it never ends as an empty success.
    await ctx.page.locator(".quote").first.wait_for(state="attached")
    for quote in await ctx.page.locator(".quote").all():
        await ctx.push(
            quotes,
            Quote(
                text=await quote.locator(".text").text_content(),
                author=await quote.locator(".author").text_content(),
                tags=await quote.locator(".tag").all_text_contents(),
                version=ctx.request.label.name,
            ),
        )
    # The Frontier fetches each URL once, so each page's quotes are pushed once.
    await ctx.enqueue_links(ctx.page.locator("li.next a"), ctx.request.label)


@crawl.handle(scroll)
async def read_scrolling_quotes(ctx: AsyncCrawlContext[None]) -> None:
    page = ctx.page
    # Page 1 loaded while the engine navigated: the Label kept it.
    loaded = await ctx.kept_response(api_page(1))
    for n in itertools.count(2):
        # A body that is not JSON raises ResponseNotJSONError.
        body = QuotesPage.model_validate(await loaded.json())
        for quote in body.quotes:
            await ctx.push(
                quotes,
                Quote(
                    text=quote.text,
                    author=quote.author.name,
                    tags=quote.tags,
                    version="scroll",
                ),
            )
        if not body.has_next:
            return
        # Each JSON page is awaited once, by its number; one that never loads fails the request with
        # DeadlineExceededError, retried, then `failed`.
        async with page.expect_response(api_page(n)) as next_page:
            await page.mouse.wheel(0, 10_000)
        loaded = await next_page.value


async def collect_quotes() -> list[Quote]:
    # No browser on the machine: the pinned one is fetched on first use.
    async with (
        AsyncFoxhound.connect() as foxhound,
        await foxhound.start_crawl(
            crawl,
            seeds=[
                seed("https://quotes.toscrape.com/", html),
                seed("https://quotes.toscrape.com/js/", js),
                seed("https://quotes.toscrape.com/scroll", scroll),
            ],
        ) as run,
    ):
        totals = (await run.finish()).requests
        print(
            f"{totals.succeeded} pages succeeded, {totals.failed} failed",
            dict(totals.by_error_code),
        )
        return [record.value async for record in run.records(quotes)]


if __name__ == "__main__":
    for found in asyncio.run(collect_quotes()):
        print(f"[{found.version}] {found.text} ({found.author})")
