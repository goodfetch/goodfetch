"""One crawl with Scrapy's default HTTP crawl, for `just compare`.

ADR 0138's matrix: from `--url`, at `--concurrency` requests at once, every
page of its origin, with `--host` sent as each request's `Host` so the Proving
ground serves its family on the loopback address, nothing kept on disk
(scenario 1; scenario 2's cell says why Scrapy has none). It prints one line,
`pages=<n>`, the pages Scrapy says it parsed; the comparison's same-work check
reads the Proving ground's own observations instead.

    python crawl.py --url http://127.0.0.1:<port>/sites/1000/1 \
      --host scale.test:<port> --concurrency 16
"""

from __future__ import annotations

import argparse
import sys
from typing import TYPE_CHECKING, Any
from urllib.parse import urlsplit

import scrapy
from scrapy.crawler import CrawlerProcess

if TYPE_CHECKING:
    from collections.abc import AsyncIterator, Iterator

    from scrapy.http import Response


class Site(scrapy.Spider):
    """Follows every link of the site's origin, parsing each page once."""

    name = "site"

    def __init__(self, url: str, **kwargs: Any) -> None:  # noqa: ANN401
        """Starts at `url`; Scrapy's offsite filter keeps the crawl on its host."""
        super().__init__(**kwargs)
        self.url = url
        self.allowed_domains = [urlsplit(url).hostname or ""]
        self.pages = 0

    async def start(self) -> AsyncIterator[scrapy.Request]:
        """The first page, through the duplicate filter as every other request.

        Scrapy's `start_urls` skip the filter, so the site's links back to its
        first page would fetch it a second time.
        """
        yield scrapy.Request(self.url, callback=self.parse)

    def parse(self, response: Response, **_kwargs: Any) -> Iterator[scrapy.Request]:  # noqa: ANN401
        """Counts the page and follows each of its links."""
        self.pages += 1
        yield from response.follow_all(css="a", callback=self.parse)


def main() -> int:
    """Runs the crawl and prints the pages Scrapy parsed."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", required=True)
    parser.add_argument("--host", required=True)
    parser.add_argument("--concurrency", required=True, type=int)
    arguments = parser.parse_args()
    settings: dict[str, Any] = {
        "ROBOTSTXT_OBEY": False,
        "CONCURRENT_REQUESTS": arguments.concurrency,
        "CONCURRENT_REQUESTS_PER_DOMAIN": arguments.concurrency,
        "DOWNLOAD_DELAY": 0,
        "DEFAULT_REQUEST_HEADERS": {"Host": arguments.host},
        "LOG_LEVEL": "INFO",
    }
    process = CrawlerProcess(settings)
    crawler = process.create_crawler(Site)
    process.crawl(crawler, url=arguments.url)
    process.start()
    spider = crawler.spider
    if not isinstance(spider, Site):
        sys.stderr.write("crawl.py: the crawl never started\n")
        return 1
    sys.stdout.write(f"pages={spider.pages}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
