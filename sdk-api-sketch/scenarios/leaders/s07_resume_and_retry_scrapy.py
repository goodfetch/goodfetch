"""Golden scenario 07 on Scrapy 2.19.0: scenario 05's spider under a JOBDIR named by the program. Stop it with one
Ctrl+C (a second one, or a SIGKILL, loses what was in flight) and run it again to resume; once idle, it retries the
failed requests once; a third run finds everything already seen and says so (`python s07_resume_and_retry_scrapy.py`).
"""

from typing import Any

import scrapy
from scrapy import signals
from scrapy.crawler import Crawler, CrawlerProcess
from scrapy.exceptions import DontCloseSpider, IgnoreRequest
from scrapy.spidermiddlewares.httperror import HttpError
from scrapy.utils.asyncio import create_looping_call
from twisted.python.failure import Failure

from s05_two_page_crawl_scrapy import POLITE, BooksSpider

JOBDIR = "crawls/books-toscrape-2026-10-15"


class ResumableBooksSpider(BooksSpider):
    name = "books-resumable"
    custom_settings = {
        **POLITE,
        "JOBDIR": JOBDIR,
        # Appended to on each run: a JSON array cannot be continued after a stop.
        "FEEDS": {"books.jsonl": {"format": "jsonlines", "encoding": "utf8"}},
    }
    state: dict[str, Any]  # set by Scrapy's SpiderState extension, kept in JOBDIR
    progress: Any

    @classmethod
    def from_crawler(
        cls, crawler: Crawler, *args: Any, **kwargs: Any
    ) -> "ResumableBooksSpider":
        spider = super().from_crawler(crawler, *args, **kwargs)
        crawler.signals.connect(spider.retry_failed, signal=signals.spider_idle)
        spider.progress = create_looping_call(spider.print_progress)
        crawler.signals.connect(
            lambda: spider.progress.start(5), signal=signals.spider_opened
        )
        return spider

    def errback(self, failure: Failure) -> None:
        super().errback(failure)
        request = failure.request  # type: ignore[attr-defined]
        if failure.check(HttpError) or not failure.check(
            IgnoreRequest
        ):  # a failure, not a skip
            # `self.state` is saved in JOBDIR on a clean stop, so failures survive a resume.
            failed = self.state.setdefault("failed", [])
            failed.append((request.url, request.callback.__name__, request.cb_kwargs))

    def retry_failed(self) -> None:
        if self.state.get("retried") or not self.state.get("failed"):
            return
        self.state["retried"] = True
        failed = self.state.pop("failed")
        self.logger.info("retrying %d failed requests", len(failed))
        for url, callback, cb_kwargs in failed:
            # Already seen, so only `dont_filter` lets them through again.
            retry = scrapy.Request(
                url,
                getattr(self, callback),
                cb_kwargs=cb_kwargs,
                errback=self.errback,
                dont_filter=True,
            )
            self.crawler.engine.crawl(retry)
        raise DontCloseSpider

    def print_progress(self) -> None:
        stats, downloader = self.crawler.stats, self.crawler.engine.downloader
        pending = stats.get_value("scheduler/enqueued", 0) - stats.get_value(
            "scheduler/dequeued", 0
        )
        print(f"{pending} pending, {len(downloader.active)} in flight")
        for domain, slot in downloader.slots.items():
            print(
                f"  {domain}: {len(slot.active)} active, {len(slot.queue)} waiting, delay {slot.delay}s"
            )

    def closed(self, reason: str) -> None:
        self.progress.stop()
        super().closed(reason)
        stats = self.crawler.stats
        if not stats.get_value("item_scraped_count") and not self.failures:
            filtered = stats.get_value("dupefilter/filtered", 0)
            print(
                f"nothing done: {filtered} links were already seen in {JOBDIR}/requests.seen"
            )


if __name__ == "__main__":
    process = CrawlerProcess()
    process.crawl(ResumableBooksSpider)
    process.start()
