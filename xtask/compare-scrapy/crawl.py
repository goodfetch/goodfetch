"""One crawl with Scrapy, for `just compare`.

ADR 0138's matrix: from `--url`, at `--concurrency` requests at once, every
page of the site, with `--host`, when given, sent as each request's `Host` so
the Proving ground serves its family on the loopback address, nothing kept on
disk but the extraction scenario's items. `--scenario` names the matrix's
scenario:

- `raw`: Scrapy's default crawl on the first page's host, no delay,
  robots.txt not obeyed;
- `polite`: robots.txt obeyed, one request in flight and a second between
  requests on each domain, every link followed, since the multi-domain site
  spans many hosts and links nothing else;
- `extraction`: `raw` with each page's title and main text as an Item, which
  an Item pipeline writes as JSON lines to `items.jsonl`;
- `browser`: `raw` with every request through `scrapy-playwright`, in the
  headless Chrome `--browser` names;
- `hostile`: the hostile pages' pass at each tool's defaults (#2439): no
  setting but the `Host` header and `LOG_LEVEL: INFO`, which changes only
  what Scrapy logs, so robots.txt is not obeyed (`ROBOTSTXT_OBEY` is False
  by default), 16 requests in flight, 8 a domain, no delay, and 429s and
  503s retried twice; `--concurrency` is unread.

Every scenario but `hostile` verifies an `https` site's certificate against
the platform's roots, which `SSL_CERT_FILE` extends with the Proving
ground's authority, and fetches it over HTTP/2 unless in the browser: the
crawl-like scenario (#2284) runs `raw` over TLS and HTTP/2.

Scenario 2's cell says why Scrapy has none. It prints one line, `pages=<n>`,
the pages Scrapy says it parsed; the comparison's same-work check reads the
Proving ground's own observations instead.

    python crawl.py --url http://127.0.0.1:<port>/sites/1000/1 \
      --host scale.test:<port> --concurrency 16 --scenario raw
"""

from __future__ import annotations

import argparse
import json
import sys
from typing import IO, TYPE_CHECKING, Any
from urllib.parse import urlsplit

import scrapy
from scrapy.crawler import CrawlerProcess

if TYPE_CHECKING:
    from collections.abc import AsyncIterator, Iterator

    from scrapy.http import Response


SCENARIOS = ("raw", "polite", "extraction", "browser", "hostile")

ITEMS_FILE = "items.jsonl"

REACTOR = "twisted.internet.asyncioreactor.AsyncioSelectorReactor"

# Scrapy's HTTP/2 download handler, which offers `h2` alone over ALPN.
H2_HANDLER = "scrapy.core.downloader.handlers.http2.H2DownloadHandler"


class Site(scrapy.Spider):
    """Follows every link of the site, parsing each page once."""

    name = "site"

    def __init__(self, url: str, scenario: str) -> None:
        """Starts at `url`.

        Scrapy's offsite filter keeps the crawl on its host, but for the
        multi-domain site, whose every page links others on other hosts.
        `main` passes these two arguments alone, so no other reaches
        Scrapy's own.
        """
        super().__init__()
        self.url = url
        self.scenario = scenario
        if scenario != "polite":
            self.allowed_domains = [urlsplit(url).hostname or ""]
        self.meta = {"playwright": True} if scenario == "browser" else {}
        self.pages = 0

    async def start(self) -> AsyncIterator[scrapy.Request]:
        """The first page, through the duplicate filter as every other request.

        Scrapy's `start_urls` skip the filter, so the site's links back to its
        first page would fetch it a second time.
        """
        yield scrapy.Request(self.url, callback=self.parse, meta=self.meta)

    def parse(
        self,
        response: Response,
        **_kwargs: object,
    ) -> Iterator[scrapy.Request | dict[str, str]]:
        """Counts the page, yields its Item in `extraction`, and follows each link."""
        self.pages += 1
        if self.scenario == "extraction":
            yield {
                "url": response.url,
                "title": response.css("title::text").get(default=""),
                "text": " ".join(response.css("main ::text").getall()).strip(),
            }
        yield from response.follow_all(css="a", callback=self.parse, meta=self.meta)


class JsonLines:
    """The Item pipeline that writes each Item as one JSON line."""

    def __init__(self) -> None:
        """Opens nothing until the crawl starts."""
        self.file: IO[str] | None = None

    def open_spider(self) -> None:
        """Opens the items' file."""
        self.file = open(ITEMS_FILE, "w", encoding="utf-8")

    def close_spider(self) -> None:
        """Closes the items' file."""
        if self.file is not None:
            self.file.close()

    def process_item(self, item: dict[str, str]) -> dict[str, str]:
        """Writes `item` as one line."""
        if self.file is not None:
            self.file.write(json.dumps(item) + "\n")
        return item


def settings_of(arguments: argparse.Namespace) -> dict[str, Any]:
    """The scenario's settings."""
    if arguments.scenario == "hostile":
        defaults: dict[str, Any] = {"LOG_LEVEL": "INFO"}
        if arguments.host:
            defaults["DEFAULT_REQUEST_HEADERS"] = {"Host": arguments.host}
        return defaults
    settings: dict[str, Any] = {
        "ROBOTSTXT_OBEY": False,
        "CONCURRENT_REQUESTS": arguments.concurrency,
        "CONCURRENT_REQUESTS_PER_DOMAIN": arguments.concurrency,
        "DOWNLOAD_DELAY": 0,
        "LOG_LEVEL": "INFO",
        # Scrapy accepts any certificate by default.
        "DOWNLOAD_VERIFY_CERTIFICATES": True,
        "DOWNLOAD_HANDLERS": {"https": H2_HANDLER},
    }
    if arguments.host:
        settings["DEFAULT_REQUEST_HEADERS"] = {"Host": arguments.host}
    if arguments.scenario == "polite":
        settings.update(
            {
                "ROBOTSTXT_OBEY": True,
                "DOWNLOAD_DELAY": 1,
                "RANDOMIZE_DOWNLOAD_DELAY": False,
                "CONCURRENT_REQUESTS_PER_DOMAIN": 1,
            },
        )
    if arguments.scenario == "extraction":
        settings["ITEM_PIPELINES"] = {f"{__name__}.JsonLines": 300}
    if arguments.scenario == "browser":
        handler = "scrapy_playwright.handler.ScrapyPlaywrightDownloadHandler"
        settings.update(
            {
                "DOWNLOAD_HANDLERS": {"http": handler, "https": handler},
                "TWISTED_REACTOR": REACTOR,
                "PLAYWRIGHT_BROWSER_TYPE": "chromium",
                "PLAYWRIGHT_LAUNCH_OPTIONS": {
                    "executable_path": arguments.browser,
                    "headless": True,
                    "args": [
                        "--use-mock-keychain",
                        "--password-store=basic",
                        "--no-first-run",
                    ],
                },
            },
        )
    return settings


def main() -> int:
    """Runs the crawl and prints the pages Scrapy parsed."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", required=True)
    parser.add_argument("--host")
    parser.add_argument("--concurrency", required=True, type=int)
    parser.add_argument("--scenario", required=True, choices=SCENARIOS)
    parser.add_argument("--browser")
    arguments = parser.parse_args()
    if arguments.scenario == "browser" and not arguments.browser:
        parser.error("the browser scenario takes --browser")
    process = CrawlerProcess(settings_of(arguments))
    crawler = process.create_crawler(Site)
    process.crawl(crawler, url=arguments.url, scenario=arguments.scenario)
    process.start()
    spider = crawler.spider
    if not isinstance(spider, Site):
        sys.stderr.write("crawl.py: the crawl never started\n")
        return 1
    sys.stdout.write(f"pages={spider.pages}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
