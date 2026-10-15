"""Collect every quote of the three versions of quotes.toscrape.com with one Foxhound crawl.

* The plain HTML version is fetched over plain HTTP (the `http` tier is forced for its Label, so a page without
  quotes can never be counted as a success: waiting for the quotes raises `BrowserRequiredError` instead).
* The JavaScript-rendered version needs a browser, so its Label forces the `browser` tier. It shares the plain
  version's handler: the same function reads the quotes from whatever document the tier produced.
* The infinite-scrolling version runs in a browser too, and its quotes are taken from the JSON responses of
  `/api/quotes` that the page loads, never from the screen.
* In the browser, images and fonts are never downloaded (a route registered before the navigation aborts them).
"""

from __future__ import annotations

import asyncio
from typing import Final

from pydantic import BaseModel

from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncCrawlContext,
    AsyncFoxhound,
    AsyncPageStageContext,
    AsyncResponse,
    AsyncRoute,
    Count,
    Dataset,
    Label,
    LabelSettings,
    seed,
)

PLAIN_URL: Final = "https://quotes.toscrape.com/"
RENDERED_URL: Final = "https://quotes.toscrape.com/js/"
SCROLL_URL: Final = "https://quotes.toscrape.com/scroll"

_BLOCKED_RESOURCE_TYPES: Final = frozenset({"image", "font"})
_API_PATH: Final = "/api/quotes"
_PAGE_WAIT_SECONDS: Final = 30.0


class Quote(BaseModel):
    """One quote of one version of the site."""

    version: str
    """`html`, `js` or `scroll`."""
    text: str
    author: str
    tags: list[str]


class _ApiAuthor(BaseModel):
    name: str


class _ApiQuote(BaseModel):
    text: str
    author: _ApiAuthor
    tags: list[str]


class _ApiPage(BaseModel):
    """One JSON response of the scrolling version's `/api/quotes?page=N`."""

    page: int
    has_next: bool
    quotes: list[_ApiQuote]


class _ScrollCapture:
    """The API pages one scroll request's browser page has loaded so far."""

    def __init__(self) -> None:
        self._pages: dict[int, _ApiPage] = {}
        self._arrived = asyncio.Event()

    def add(self, api_page: _ApiPage) -> None:
        self._pages[api_page.page] = api_page
        self._arrived.set()

    async def page(self, number: int) -> _ApiPage:
        while number not in self._pages:
            self._arrived.clear()
            if number in self._pages:
                break
            await self._arrived.wait()
        return self._pages[number]


crawl = AsyncCrawl()

plain: Label[None] = crawl.label("plain", settings=LabelSettings(fetch={"tier": "http"}))
rendered: Label[None] = crawl.label("rendered", settings=LabelSettings(fetch={"tier": "browser"}))
scroll: Label[None] = crawl.label("scroll", settings=LabelSettings(fetch={"tier": "browser"}))

quotes: Dataset[Quote] = crawl.dataset("quotes", schema=Quote, key=["version", "text"])

_VERSION_OF_LABEL: Final = {plain.name: "html", rendered.name: "js"}
_captures: dict[str, _ScrollCapture] = {}


async def collect_quotes(ctx: AsyncCrawlContext[None]) -> None:
    """The one handler of the plain and the rendered versions: quotes from the document, then the next page."""
    page = ctx.page
    version = _VERSION_OF_LABEL[ctx.request.label.name]
    # A document without quotes (the rendered version fetched over HTTP) cannot satisfy this wait: the attempt
    # escalates to the browser, or raises `BrowserRequiredError` when the tier is forced. It is never a success.
    await page.locator(".quote").wait_for_count(Count(min=1))
    found = await page.locator(".quote").all()
    if not found:
        ctx.retry("the page holds no quotes")
    for element in found:
        text = await element.locator(".text").text_content()
        author = await element.locator(".author").text_content()
        tags = await element.locator(".tags .tag").all_text_contents()
        await ctx.push(quotes, Quote(version=version, text=text.strip(), author=author.strip(), tags=[t.strip() for t in tags]))
    await ctx.enqueue_links(page.locator("li.next a"), ctx.request.label)


crawl.handle(plain)(collect_quotes)
crawl.handle(rendered)(collect_quotes)


@crawl.before_navigation(labels=[rendered, scroll], name="block_images_and_fonts")
async def block_images_and_fonts(ctx: AsyncPageStageContext[None]) -> None:
    async def answer(route: AsyncRoute) -> None:
        if route.request.resource_type in _BLOCKED_RESOURCE_TYPES:
            await route.abort("blockedbyclient")
        else:
            await route.continue_()

    ctx.page.route("**/*", answer)


@crawl.before_navigation(labels=[scroll], name="capture_quote_responses")
async def capture_quote_responses(ctx: AsyncPageStageContext[None]) -> None:
    capture = _ScrollCapture()
    _captures[ctx.request.id] = capture

    async def on_response(response: AsyncResponse) -> None:
        if _API_PATH not in response.url or not response.ok:
            return
        capture.add(_ApiPage.model_validate(await response.json()))

    ctx.page.on("response", on_response)


@crawl.handle(scroll)
async def collect_scrolled_quotes(ctx: AsyncCrawlContext[None]) -> None:
    """The scrolling version: every quote comes from a JSON response the page loaded while it was scrolled."""
    capture = _captures.get(ctx.request.id)
    if capture is None:
        ctx.retry("the page's responses were not captured")
    page = ctx.page
    seen = 0
    number = 1
    try:
        async with asyncio.timeout(_PAGE_WAIT_SECONDS * 20):
            while True:
                try:
                    async with asyncio.timeout(_PAGE_WAIT_SECONDS):
                        api_page = await capture.page(number)
                except TimeoutError:
                    ctx.retry(f"the quotes of page {number} never arrived")
                for item in api_page.quotes:
                    await ctx.push(
                        quotes, Quote(version="scroll", text=item.text, author=item.author.name, tags=item.tags)
                    )
                seen += len(api_page.quotes)
                if not api_page.has_next:
                    break
                # Let the page render what it received, then scroll to the bottom to make it ask for the next page.
                await page.locator(".quote").wait_for_count(Count(min=seen))
                await page.evaluate("window.scrollTo(0, document.body.scrollHeight)")
                number += 1
    except TimeoutError:
        ctx.retry("scrolling did not finish in time")
    finally:
        _captures.pop(ctx.request.id, None)


async def main() -> list[Quote]:
    """Runs the crawl and returns the collected quotes."""
    async with AsyncFoxhound.connect() as foxhound:
        run = await foxhound.start_crawl(
            crawl,
            seeds=[seed(PLAIN_URL, plain), seed(RENDERED_URL, rendered), seed(SCROLL_URL, scroll)],
        )
        await run.finish()
        collected = [record.value async for record in run.records(quotes)]
    return collected


if __name__ == "__main__":
    asyncio.run(main())
