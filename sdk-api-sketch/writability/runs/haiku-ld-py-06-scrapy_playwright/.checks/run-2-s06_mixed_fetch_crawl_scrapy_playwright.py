"""Collect every quote from the three quotes.toscrape.com versions.

- /        Content is in the HTML: fetched over plain HTTP, following li.next.
- /js/     Rendered by JavaScript: fetched in the browser by the same spider and
           the same listing handler, following li.next.
- /scroll  Infinite scroll: the browser scrolls the page and the quotes are read
           from the JSON responses the page loads, not from the screen.

Each quote is recorded once per version. A page whose quotes are missing is
never counted as a success: a plain HTTP page without quotes is retried once in
the browser, and a page that still has no quotes is recorded as a failure.
"""

import logging
from collections import Counter
from collections.abc import AsyncIterator, Iterator
from dataclasses import dataclass
from typing import Any
from urllib.parse import urlparse

import scrapy
from parsel import Selector
from playwright.async_api import Error as PlaywrightError
from playwright.async_api import Page
from playwright.async_api import Request as PlaywrightRequest
from playwright.async_api import Response as PlaywrightResponse
from scrapy import Request
from scrapy.crawler import CrawlerProcess
from scrapy.http import Response
from scrapy_playwright.page import PageMethod
from twisted.python.failure import Failure

logger = logging.getLogger(__name__)

BASE_URL = "https://quotes.toscrape.com"
HTML_VERSION = "html"
JS_VERSION = "js"
SCROLL_VERSION = "scroll"
QUOTES_SELECTOR = "div.quote"
QUOTES_API_PATH = "/api/quotes"
WAIT_FOR_QUOTES_MS = 15_000
SCROLL_TICK_MS = 250
SCROLL_WAIT_TICKS = 40  # about 10 seconds for the next JSON page to arrive
MAX_SCROLL_STEPS = 50
OUTPUT_FILE = "quotes.jsonl"
QUOTE_VERSION_KEY = "quote_version"
QUOTE_CAPTURE_KEY = "quote_capture"


@dataclass(frozen=True)
class Quote:
    version: str
    text: str
    author: str
    tags: tuple[str, ...]


def abort_images_and_fonts(request: PlaywrightRequest) -> bool:
    return request.resource_type in ("image", "font")


def _quote_from_html(block: Selector, version: str) -> Quote | None:
    text = block.css("span.text::text").get()
    author = block.css("small.author::text").get()
    if text is None or author is None:
        return None
    tags = tuple(tag.strip() for tag in block.css("div.tags a.tag::text").getall())
    return Quote(version, text.strip(), author.strip(), tags)


def _quote_from_json(raw: object, version: str) -> Quote | None:
    if not isinstance(raw, dict):
        return None
    text = raw.get("text")
    author = raw.get("author")
    if isinstance(author, dict):
        author = author.get("name")
    tags = raw.get("tags")
    if not isinstance(text, str) or not isinstance(author, str):
        return None
    tag_names = tuple(tag for tag in tags if isinstance(tag, str)) if isinstance(tags, list) else ()
    return Quote(version, text.strip(), author.strip(), tag_names)


def _listing_meta(version: str, *, browser: bool) -> dict[str, Any]:
    meta: dict[str, Any] = {QUOTE_VERSION_KEY: version}
    if browser:
        meta["playwright"] = True
        meta["playwright_page_methods"] = [
            PageMethod("wait_for_selector", QUOTES_SELECTOR, timeout=WAIT_FOR_QUOTES_MS)
        ]
    return meta


class QuoteCapture:
    """Records the quote JSON responses a scroll page loads and reads them while scrolling."""

    def __init__(self, version: str) -> None:
        self.version = version
        self.responses: list[PlaywrightResponse] = []
        self.quotes: list[Quote] = []

    def on_response(self, response: PlaywrightResponse) -> None:
        if urlparse(response.url).path == QUOTES_API_PATH:
            self.responses.append(response)

    async def scroll_until_exhausted(self, page: Page) -> None:
        consumed = 0
        for _ in range(MAX_SCROLL_STEPS):
            if not await self._wait_for_new_response(page, consumed):
                return
            fresh = self.responses[consumed:]
            consumed = len(self.responses)
            finished = False
            for response in fresh:
                if await self._consume(response):
                    finished = True
            if finished:
                return
            await page.evaluate("window.scrollTo(0, document.body.scrollHeight)")

    async def _wait_for_new_response(self, page: Page, consumed: int) -> bool:
        for _ in range(SCROLL_WAIT_TICKS):
            if len(self.responses) > consumed:
                return True
            await page.wait_for_timeout(SCROLL_TICK_MS)
        return len(self.responses) > consumed

    async def _consume(self, response: PlaywrightResponse) -> bool:
        """Record the quotes of one JSON response; return True when it is the last page."""
        try:
            payload: object = await response.json()
        except (PlaywrightError, ValueError) as error:
            logger.error("Could not read quote JSON from %s: %s", response.url, error)
            return False
        if not isinstance(payload, dict):
            logger.error("Quote JSON from %s is not an object", response.url)
            return False
        is_last_page = payload.get("has_next") is False
        raw_quotes = payload.get("quotes")
        if not isinstance(raw_quotes, list):
            logger.error("Quote JSON from %s has no quotes list", response.url)
            return is_last_page
        for raw in raw_quotes:
            quote = _quote_from_json(raw, self.version)
            if quote is None:
                logger.error("Skipping a malformed quote in %s", response.url)
            else:
                self.quotes.append(quote)
        return is_last_page


class QuotesSpider(scrapy.Spider):
    name = "quotes_mixed"

    def __init__(self, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self._seen: set[tuple[str, str, str]] = set()
        self._counts: Counter[str] = Counter()
        self.failures: list[str] = []

    async def start(self) -> AsyncIterator[Request]:
        yield self._listing_request(HTML_VERSION, f"{BASE_URL}/", browser=False)
        yield self._listing_request(JS_VERSION, f"{BASE_URL}/js/", browser=True)
        yield self._scroll_request()

    def parse_listing(self, response: Response) -> Iterator[Request | Quote]:
        version = str(response.meta[QUOTE_VERSION_KEY])
        blocks = response.css(QUOTES_SELECTOR)
        if not blocks:
            if response.meta.get("playwright"):
                self._fail(response.url, "no quotes in the rendered page")
            else:
                self.logger.warning(
                    "No quotes in the HTTP response for %s, retrying it in the browser",
                    response.url,
                )
                yield self._listing_request(version, response.url, browser=True, dont_filter=True)
            return
        for block in blocks:
            quote = _quote_from_html(block, version)
            if quote is None:
                self.logger.error("Skipping a quote without text or author on %s", response.url)
            elif self._record(quote):
                yield quote
        next_href = response.css("li.next a::attr(href)").get()
        if next_href is not None:
            # Pagination follows the version's own fetch mode: only /js/ is browser-rendered.
            yield response.follow(
                next_href,
                callback=self.parse_listing,
                errback=self.on_error,
                meta=_listing_meta(version, browser=version == JS_VERSION),
            )

    def parse_scroll(self, response: Response) -> Iterator[Quote]:
        capture: QuoteCapture = response.meta[QUOTE_CAPTURE_KEY]
        if not capture.quotes:
            self._fail(response.url, "no quotes in the JSON responses the page loaded")
        for quote in capture.quotes:
            if self._record(quote):
                yield quote

    def on_error(self, failure: Failure) -> None:
        request: Request | None = getattr(failure, "request", None)
        url = request.url if request is not None else "unknown URL"
        self._fail(url, failure.getErrorMessage())

    def closed(self, reason: str) -> None:
        self.logger.info(
            "Quotes recorded per version: %s; failed pages: %d",
            dict(self._counts),
            len(self.failures),
        )

    def _listing_request(
        self, version: str, url: str, *, browser: bool, dont_filter: bool = False
    ) -> Request:
        return Request(
            url,
            callback=self.parse_listing,
            errback=self.on_error,
            dont_filter=dont_filter,
            meta=_listing_meta(version, browser=browser),
        )

    def _scroll_request(self) -> Request:
        capture = QuoteCapture(SCROLL_VERSION)
        return Request(
            f"{BASE_URL}/scroll",
            callback=self.parse_scroll,
            errback=self.on_error,
            dont_filter=True,
            meta={
                QUOTE_VERSION_KEY: SCROLL_VERSION,
                QUOTE_CAPTURE_KEY: capture,
                "playwright": True,
                "playwright_page_event_handlers": {"response": capture.on_response},
                "playwright_page_methods": [PageMethod(capture.scroll_until_exhausted)],
            },
        )

    def _record(self, quote: Quote) -> bool:
        key = (quote.version, quote.author, quote.text)
        if key in self._seen:
            return False
        self._seen.add(key)
        self._counts[quote.version] += 1
        return True

    def _fail(self, url: str, reason: str) -> None:
        self.failures.append(f"{url}: {reason}")
        self.logger.error("Page not counted as a success: %s (%s)", url, reason)


SETTINGS: dict[str, Any] = {
    "TWISTED_REACTOR": "twisted.internet.asyncioreactor.AsyncioSelectorReactor",
    "DOWNLOAD_HANDLERS": {
        "http": "scrapy_playwright.handler.ScrapyPlaywrightDownloadHandler",
        "https": "scrapy_playwright.handler.ScrapyPlaywrightDownloadHandler",
    },
    "PLAYWRIGHT_BROWSER_TYPE": "chromium",
    "PLAYWRIGHT_LAUNCH_OPTIONS": {"headless": True},
    "PLAYWRIGHT_ABORT_REQUEST": abort_images_and_fonts,
    "FEEDS": {OUTPUT_FILE: {"format": "jsonlines", "overwrite": True}},
    "LOG_LEVEL": "INFO",
}


def main() -> None:
    process = CrawlerProcess(SETTINGS)
    process.crawl(QuotesSpider)
    process.start()


if __name__ == "__main__":
    main()
