"""Collect every quote from the three versions of quotes.toscrape.com with one Scrapy crawler.

- https://quotes.toscrape.com/        plain HTML: fetched over plain HTTP.
- https://quotes.toscrape.com/js/     rendered by JavaScript: fetched in the browser.
- https://quotes.toscrape.com/scroll  infinite scrolling: fetched in the browser, the quotes
  are taken from the JSON responses (/api/quotes?page=N) the page loads while it scrolls.

scrapy-playwright's download handler serves both schemes. A request without
``meta["playwright"]`` goes over plain HTTP, so one spider and one set of callbacks serve the
HTTP pages and the browser pages. The browser aborts images and fonts.
"""

from __future__ import annotations

import asyncio
import sys
from collections.abc import AsyncIterator, Iterator
from typing import Any

from playwright.async_api import Page
from playwright.async_api import Request as PlaywrightRequest
from playwright.async_api import Response as PlaywrightResponse
from scrapy import Request, Spider
from scrapy.crawler import CrawlerProcess
from scrapy.http import Response, TextResponse
from scrapy_playwright.page import PageMethod
from twisted.python.failure import Failure

BASE = "https://quotes.toscrape.com"
SCROLL_STEP_TIMEOUT_S = 15.0
BLOCKED_RESOURCE_TYPES = frozenset({"image", "font"})

VERSION_HTML = "html"
VERSION_JS = "js"
VERSION_SCROLL = "scroll"


def abort_images_and_fonts(request: PlaywrightRequest) -> bool:
    """PLAYWRIGHT_ABORT_REQUEST: the browser downloads neither images nor fonts."""
    return request.resource_type in BLOCKED_RESOURCE_TYPES


class ScrollCollector:
    """Gathers the quotes of the JSON responses a scrolling page loads."""

    def __init__(self) -> None:
        self.quotes: list[dict[str, Any]] = []
        self.pages_seen: set[int] = set()
        self.has_next = True
        self.last_page = 0
        self.changed = asyncio.Event()

    async def on_response(self, response: PlaywrightResponse) -> None:
        if "/api/quotes" not in response.url or not response.ok:
            return
        try:
            payload = await response.json()
        except Exception:  # body gone because the page navigated or closed
            return
        if not isinstance(payload, dict):
            return
        page_number = payload.get("page")
        quotes = payload.get("quotes")
        if not isinstance(page_number, int) or not isinstance(quotes, list):
            return
        if page_number in self.pages_seen:
            return
        self.pages_seen.add(page_number)
        self.quotes.extend(q for q in quotes if isinstance(q, dict))
        self.last_page = max(self.last_page, page_number)
        self.has_next = bool(payload.get("has_next"))
        self.changed.set()


async def scroll_until_exhausted(page: Page, collector: ScrollCollector) -> None:
    """PageMethod callable: scroll to the bottom until the API reports no next page."""
    # The first JSON response is requested by the page itself while it loads.
    if not collector.pages_seen:
        await asyncio.wait_for(_next_change(collector), SCROLL_STEP_TIMEOUT_S)
    while collector.has_next:
        seen_before = len(collector.pages_seen)
        await page.evaluate("window.scrollTo(0, document.body.scrollHeight)")
        deadline = asyncio.get_running_loop().time() + SCROLL_STEP_TIMEOUT_S
        while len(collector.pages_seen) == seen_before:
            remaining = deadline - asyncio.get_running_loop().time()
            if remaining <= 0:
                raise TimeoutError(
                    f"scrolling did not load a page after page {collector.last_page}"
                )
            await asyncio.wait_for(_next_change(collector), remaining)


async def _next_change(collector: ScrollCollector) -> None:
    collector.changed.clear()
    await collector.changed.wait()


def _clean_text(text: str) -> str:
    return text.strip().strip("“”").strip()


class QuotesSpider(Spider):
    name = "quotes_three_versions"

    custom_settings = {
        "DOWNLOAD_HANDLERS": {
            "http": "scrapy_playwright.handler.ScrapyPlaywrightDownloadHandler",
            "https": "scrapy_playwright.handler.ScrapyPlaywrightDownloadHandler",
        },
        "TWISTED_REACTOR": "twisted.internet.asyncioreactor.AsyncioSelectorReactor",
        "PLAYWRIGHT_BROWSER_TYPE": "chromium",
        "PLAYWRIGHT_LAUNCH_OPTIONS": {"headless": True},
        "PLAYWRIGHT_ABORT_REQUEST": abort_images_and_fonts,
        "CONCURRENT_REQUESTS": 4,
        "DOWNLOAD_DELAY": 0.25,
        "FEEDS": {"quotes.jsonl": {"format": "jsonlines", "overwrite": True}},
    }

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        super().__init__(*args, **kwargs)
        self._recorded: set[tuple[str, str, str]] = set()

    # -- requests -------------------------------------------------------------------------

    def _http_request(self, url: str, version: str) -> Request:
        return Request(
            url,
            callback=self.parse_listing,
            errback=self.on_failure,
            cb_kwargs={"version": version},
        )

    def _browser_request(self, url: str, version: str) -> Request:
        return Request(
            url,
            callback=self.parse_listing,
            errback=self.on_failure,
            cb_kwargs={"version": version},
            meta={
                "playwright": True,
                "playwright_page_methods": [PageMethod("wait_for_selector", "div.quote")],
            },
        )

    def _scroll_request(self, url: str) -> Request:
        collector = ScrollCollector()
        return Request(
            url,
            callback=self.parse_scroll,
            errback=self.on_failure,
            meta={
                "playwright": True,
                "scroll_collector": collector,
                "playwright_page_event_handlers": {"response": collector.on_response},
                "playwright_page_methods": [PageMethod(scroll_until_exhausted, collector)],
            },
        )

    async def start(self) -> AsyncIterator[Any]:
        yield self._http_request(f"{BASE}/", VERSION_HTML)
        yield self._browser_request(f"{BASE}/js/", VERSION_JS)
        yield self._scroll_request(f"{BASE}/scroll")

    # -- parsing --------------------------------------------------------------------------

    def _record(self, version: str, text: str, author: str, tags: list[str]) -> Iterator[dict[str, Any]]:
        key = (version, text, author)
        if key in self._recorded:
            return
        self._recorded.add(key)
        yield {"version": version, "text": text, "author": author, "tags": tags}

    def parse_listing(self, response: Response, version: str) -> Iterator[Request | dict[str, Any]]:
        """Shared by the plain HTML pages (HTTP) and the JavaScript pages (browser)."""
        if not isinstance(response, TextResponse):
            self._page_failed(response.url, "not a text response")
            return
        blocks = response.css("div.quote")
        if not blocks:
            self._page_failed(response.url, f"no quote in the {version} page")
            return
        for block in blocks:
            text = _clean_text(block.css("span.text::text").get(default=""))
            author = block.css("small.author::text").get(default="").strip()
            tags = [t.strip() for t in block.css("div.tags a.tag::text").getall()]
            if not text or not author:
                self._page_failed(response.url, f"incomplete quote in the {version} page")
                continue
            yield from self._record(version, text, author, tags)
        next_href = response.css("li.next a::attr(href)").get()
        if next_href:
            next_url = response.urljoin(next_href)
            if version == VERSION_JS:
                yield self._browser_request(next_url, version)
            else:
                yield self._http_request(next_url, version)

    def parse_scroll(self, response: Response) -> Iterator[dict[str, Any]]:
        collector = response.meta["scroll_collector"]
        if not isinstance(collector, ScrollCollector) or not collector.quotes:
            self._page_failed(response.url, "no quote in the JSON responses")
            return
        for quote in collector.quotes:
            author = quote.get("author")
            author_name = author.get("name") if isinstance(author, dict) else author
            text = quote.get("text")
            tags = quote.get("tags")
            if not isinstance(text, str) or not isinstance(author_name, str):
                self._page_failed(response.url, "incomplete quote in the JSON responses")
                continue
            tag_list = [t for t in tags if isinstance(t, str)] if isinstance(tags, list) else []
            yield from self._record(VERSION_SCROLL, _clean_text(text), author_name.strip(), tag_list)

    # -- failures -------------------------------------------------------------------------

    def _page_failed(self, url: str, reason: str) -> None:
        self.logger.error("page failed: %s: %s", url, reason)
        if self.crawler.stats is not None:
            self.crawler.stats.inc_value("quotes/failed_pages")

    def on_failure(self, failure: Failure) -> None:
        """A request that failed (network, HTTP status, browser) is a failed page."""
        request = getattr(failure, "request", None)
        url = request.url if isinstance(request, Request) else "unknown URL"
        self._page_failed(url, failure.getErrorMessage())


def main() -> int:
    process = CrawlerProcess()
    crawler = process.create_crawler(QuotesSpider)
    process.crawl(crawler)
    process.start()
    stats = crawler.stats
    failed = stats.get_value("quotes/failed_pages", 0) if stats is not None else 0
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
