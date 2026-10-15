"""Golden scenario 06, "Crawl with HTTP and a browser only where needed": the leader's program, scrapy-playwright
0.0.48 with Scrapy 2.19.0 (Python). Run it with `scrapy runspider s06_mixed_fetch_crawl_scrapy_playwright.py`."""

import itertools
import re
from collections.abc import AsyncIterator, Iterator
from typing import Any

import scrapy
from playwright.async_api import Page
from playwright.async_api import Request as PlaywrightRequest
from scrapy.http import Response, TextResponse
from twisted.python.failure import Failure


def should_abort_request(request: PlaywrightRequest) -> bool:
    return request.resource_type in ("image", "font")


def api_page(n: int) -> re.Pattern[str]:
    return re.compile(rf"/api/quotes\?page={n}$")


class QuotesSpider(scrapy.Spider):
    name = "quotes"
    custom_settings = {
        # Requests whose meta has `playwright` go to the browser; the others are plain Scrapy requests.
        "DOWNLOAD_HANDLERS": {
            "http": "scrapy_playwright.handler.ScrapyPlaywrightDownloadHandler",
            "https": "scrapy_playwright.handler.ScrapyPlaywrightDownloadHandler",
        },
        "PLAYWRIGHT_ABORT_REQUEST": should_abort_request,
        "FEEDS": {"quotes.jsonl": {"format": "jsonlines"}},
    }

    async def start(self) -> AsyncIterator[scrapy.Request]:
        for url, version in (
            ("https://quotes.toscrape.com/", "html"),
            ("https://quotes.toscrape.com/js/", "js"),
        ):
            yield scrapy.Request(url, cb_kwargs={"version": version})
        yield scrapy.Request(
            "https://quotes.toscrape.com/scroll",
            callback=self.parse_scroll,
            meta={"playwright": True, "playwright_include_page": True},
            errback=self.close_page,
        )

    def parse(self, response: TextResponse, version: str) -> Iterator[Any]:
        quotes = response.css("div.quote")
        if not quotes:
            if response.meta.get("playwright"):
                raise ValueError(f"no quotes on {response.url}, even in the browser")
            # No quote over HTTP: fetch the same page again, in the browser.
            meta = {**response.meta, "playwright": True}
            yield response.follow(
                response.url,
                meta=meta,
                cb_kwargs={"version": version},
                dont_filter=True,
            )
            return
        for quote in quotes:
            yield {
                "text": quote.css("span.text::text").get(),
                "author": quote.css("small.author::text").get(),
                "tags": quote.css("a.tag::text").getall(),
                "version": version,
            }
        # The next page over HTTP first again: only a page without quotes goes to the browser.
        yield from response.follow_all(css="li.next a", cb_kwargs={"version": version})

    async def parse_scroll(self, response: Response) -> AsyncIterator[dict[str, Any]]:
        page: Page = response.meta["playwright_page"]
        try:
            # Page 1 loaded during the navigation, before the callback could wait for it.
            async with page.expect_response(api_page(1)) as loaded:
                await page.reload()
            for n in itertools.count(2):
                body = await (await loaded.value).json()
                for quote in body["quotes"]:
                    yield {
                        "text": quote["text"],
                        "author": quote["author"]["name"],
                        "tags": quote["tags"],
                        "version": "scroll",
                    }
                if not body["has_next"]:
                    break
                async with page.expect_response(api_page(n)) as loaded:
                    await page.mouse.wheel(0, 10_000)
        finally:
            await page.close()

    async def close_page(self, failure: Failure) -> None:
        await failure.request.meta["playwright_page"].close()  # type: ignore[attr-defined]
