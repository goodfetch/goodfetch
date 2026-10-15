"""Golden scenario 07 on Scrapy 2.19.0: resume a killed crawl, retry only its failures, say why nothing was done.

    python s07_resume_and_retry_scrapy.py crawl NAME [--kill-after N] [--status-every S]
    python s07_resume_and_retry_scrapy.py retry NAME

A crawl lives in `crawls/NAME/`:
  journal.jsonl  every request ever found and how it ended (found / done / fail / retry), appended and flushed as it
                 happens, so a hard kill loses nothing: a request that was in flight or waiting is simply still pending.
  books.jsonl    the dataset, one book per line, written by an item pipeline that refuses a URL it already holds.

`crawl NAME` resumes: it re-issues the pending requests of the journal (the first run seeds the home page). A book that
is already in the dataset is not fetched again, so after a kill the dataset ends with every book exactly once.
`retry NAME` re-issues only the requests that failed (HTTP errors, network errors; not robots.txt or offsite skips).
When there is nothing to do, the program prints that nothing was done and why, and does not start the crawler.

While the crawl runs, the extension `InFlightReport` prints what is in flight, what waits and why every
`--status-every` seconds (default 10, 0 = off) and whenever the process receives SIGUSR1.
"""

import argparse
import json
import os
import signal
import time
from collections.abc import AsyncIterator, Iterator
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Self, cast

import scrapy
from scrapy import Spider, signals
from scrapy.crawler import Crawler, CrawlerProcess
from scrapy.http import Request, Response
from scrapy.utils.asyncio import create_looping_call
from scrapy.utils.serialize import ScrapyJSONEncoder
from twisted.python.failure import Failure

from s05_two_page_crawl_scrapy import POLITE, BooksSpider

ROOT = "https://books.toscrape.com/"
PENDING, DONE, FAILED = "pending", "done", "failed"


@dataclass
class Entry:
    kind: str  # "start", "category" or "book": which callback handles the request
    category: str
    state: str = PENDING
    why: str = ""


def _read_records(path: Path) -> tuple[list[dict[str, Any]], bool]:
    """The valid JSON-object lines of a file, and whether any line was damaged (a write cut short by a kill)."""
    if not path.exists():
        return [], False
    records: list[dict[str, Any]] = []
    damaged = False
    for line in path.read_text(encoding="utf8").splitlines():
        try:
            record = json.loads(line)
        except ValueError:
            damaged = True
            continue
        if isinstance(record, dict):
            records.append(cast(dict[str, Any], record))
        else:
            damaged = True
    return records, damaged


class CrawlStore:
    """The journal and the dataset of one named crawl."""

    def __init__(self, folder: Path) -> None:
        folder.mkdir(parents=True, exist_ok=True)
        self.name = folder.name
        self.journal_path = folder / "journal.jsonl"
        self.dataset_path = folder / "books.jsonl"
        self.entries: dict[str, Entry] = {}
        self.aliases: dict[str, str] = {}  # final URL after a redirect -> the URL the request was found under
        self.book_urls: set[str] = set()
        self._encoder = ScrapyJSONEncoder()
        self._load()
        self._journal = open(self.journal_path, "a", encoding="utf8")
        self._dataset = open(self.dataset_path, "a", encoding="utf8")

    def _load(self) -> None:
        events, damaged = _read_records(self.journal_path)
        for event in events:
            self._apply(event)
        if damaged:
            self._rewrite(self.journal_path, events)
        books, damaged = _read_records(self.dataset_path)
        unique: list[dict[str, Any]] = []
        for book in books:
            url = book.get("url")
            if isinstance(url, str) and url not in self.book_urls:
                self.book_urls.add(url)
                unique.append(book)
        if damaged or len(unique) != len(books):
            self._rewrite(self.dataset_path, unique)

    def _rewrite(self, path: Path, records: list[dict[str, Any]]) -> None:
        temporary = path.with_suffix(".tmp")
        temporary.write_text(
            "".join(self._encoder.encode(r) + "\n" for r in records), encoding="utf8"
        )
        temporary.replace(path)

    def _apply(self, event: dict[str, Any]) -> None:
        url = str(event.get("url", ""))
        match event.get("e"):
            case "found":
                self.entries.setdefault(
                    url, Entry(str(event.get("kind", "")), str(event.get("category", "")))
                )
            case "done" if url in self.entries:
                self.entries[url].state = DONE
            case "fail" if url in self.entries:
                self.entries[url].state = FAILED
                self.entries[url].why = str(event.get("why", ""))
            case "retry" if url in self.entries:
                self.entries[url].state = PENDING

    def _log(self, event: dict[str, Any]) -> None:
        self._apply(event)
        self._journal.write(json.dumps(event) + "\n")
        self._journal.flush()

    def close(self) -> None:
        self._journal.close()
        self._dataset.close()

    # --- requests -------------------------------------------------------------------------------------------
    def discover(self, url: str, kind: str, category: str) -> bool:
        """Record a newly found request; False if the crawl already knows this URL (pending, done or failed)."""
        if url in self.entries:
            return False
        self._log({"e": "found", "url": url, "kind": kind, "category": category})
        return True

    def settle(self, url: str) -> None:
        url = self.aliases.get(url, url)
        entry = self.entries.get(url)
        if entry is not None and entry.state != DONE:
            self._log({"e": "done", "url": url})

    def fail(self, url: str, why: str) -> None:
        url = self.aliases.get(url, url)
        if url in self.entries:
            self._log({"e": "fail", "url": url, "why": why})

    def requeue_failed(self) -> list[str]:
        urls = self.in_state(FAILED)
        for url in urls:
            self._log({"e": "retry", "url": url})
        return urls

    def in_state(self, state: str) -> list[str]:
        return [u for u, e in self.entries.items() if e.state == state]

    # --- dataset --------------------------------------------------------------------------------------------
    def add_book(self, book: dict[str, Any]) -> bool:
        """Append a book unless its URL is already in the dataset; flushed at once, so a kill cannot tear it."""
        url = str(book["url"])
        if url in self.book_urls:
            return False
        self._dataset.write(self._encoder.encode(book) + "\n")
        self._dataset.flush()
        self.book_urls.add(url)
        return True


def origin_url(meta: dict[str, Any], url: str) -> str:
    """The URL a request was found under: the first of its redirect chain, else its own."""
    chain = meta.get("redirect_urls")
    return str(chain[0]) if chain else url


class ResumableSpider(BooksSpider):
    """The scenario 05 spider with its feed export replaced by a journal and a dataset that survive a kill."""

    custom_settings = {
        **POLITE,
        "FEEDS": {},
        "ITEM_PIPELINES": {},  # filled below, once DatasetPipeline exists
        "EXTENSIONS": {},
    }

    def __init__(
        self, store: CrawlStore, only: set[str] | None = None, **kwargs: Any
    ) -> None:
        super().__init__(**kwargs)
        self.store = store
        self.only = only  # retry: re-issue just these pending URLs, leave other pending work for `crawl`
        self.already_collected = 0

    # --- start: the pending requests of the journal ---------------------------------------------------------
    async def start(self) -> AsyncIterator[Any]:
        for url in self.store.in_state(PENDING):
            if self.only is not None and url not in self.only:
                continue
            entry = self.store.entries[url]
            if entry.kind == "book" and url in self.store.book_urls:
                # Written to the dataset before the kill, but not yet marked done.
                self.already_collected += 1
                self.store.settle(url)
                continue
            if entry.kind == "start":
                yield Request(url, self.parse, errback=self.errback)
            elif entry.kind == "book":
                yield Request(
                    url,
                    self.parse_book,
                    cb_kwargs={"category": entry.category},
                    errback=self.errback,
                )
            else:
                yield Request(
                    url,
                    self.parse_category,
                    cb_kwargs={"category": entry.category},
                    errback=self.errback,
                )

    # --- callbacks: the scenario 05 logic, plus the journal ---------------------------------------------------
    def _note_origin(self, response: Response) -> str:
        origin = origin_url(response.meta, response.url)
        if origin != response.url:
            self.store.aliases[response.url] = origin
        return origin

    def _new_requests(
        self, outputs: Iterator[Request], default_kind: str
    ) -> Iterator[Request]:
        """Pass on the requests the journal has not seen yet, recording each one as found."""
        for request in outputs:
            kind = "book" if request.callback == self.parse_book else default_kind
            category = str(request.cb_kwargs.get("category", ""))
            if self.store.discover(request.url, kind, category):
                yield request

    def parse(self, response: Response) -> Iterator[scrapy.Request]:  # type: ignore[override]
        origin = self._note_origin(response)
        yield from self._new_requests(super().parse(response), "category")
        self.store.settle(origin)

    def parse_category(  # type: ignore[override]
        self, response: Response, category: str
    ) -> Iterator[scrapy.Request]:
        origin = self._note_origin(response)
        yield from self._new_requests(
            super().parse_category(response, category), "category"
        )
        self.store.settle(origin)

    def parse_book(  # type: ignore[override]
        self, response: Response, category: str
    ) -> Iterator[dict[str, Any]]:
        origin = self._note_origin(response)
        books = list(super().parse_book(response, category))
        if not books:
            self.store.settle(origin)  # skipped page (no price, stock or rating): nothing to retry
        # Otherwise DatasetPipeline settles the request once the book is in the dataset.
        yield from books

    def errback(self, failure: Failure) -> None:
        failed_before = len(self.failures)
        super().errback(failure)
        request: Request = failure.request  # type: ignore[attr-defined]
        origin = origin_url(request.meta, request.url)
        if len(self.failures) > failed_before:
            self.store.fail(origin, self.failures[-1][1])
        else:
            self.store.settle(origin)  # robots.txt or offsite: skipped for good, not retried

    def closed(self, reason: str) -> None:  # type: ignore[override]
        super().closed(reason)
        left = len(self.store.in_state(PENDING))
        failed = len(self.store.in_state(FAILED))
        print(
            f"crawl {self.store.name!r} ({reason}): {len(self.store.book_urls)} books in the dataset"
            f" ({self.already_collected} not fetched again), {left} requests pending, {failed} failed"
        )
        if left:
            print(f"continue with: crawl {self.store.name}")
        elif failed:
            print(f"retry the failed requests with: retry {self.store.name}")
        else:
            print("the crawl is complete")


class DatasetPipeline:
    """Writes each book to the dataset once, then marks its request done; optionally kills the process (a test aid)."""

    def __init__(self, crawler: Crawler) -> None:
        self.crawler = crawler
        self.kill_after = crawler.settings.getint("KILL_AFTER")
        self.written = 0

    @classmethod
    def from_crawler(cls, crawler: Crawler) -> Self:
        return cls(crawler)

    def process_item(self, item: Any) -> Any:
        spider = cast(ResumableSpider, self.crawler.spider)
        if spider.store.add_book(dict(item)):
            self.written += 1
        spider.store.settle(str(item["url"]))
        if self.kill_after and self.written >= self.kill_after:
            os.kill(os.getpid(), signal.SIGKILL)  # no cleanup at all, like a crash or a power cut
        return item


class InFlightReport:
    """Prints what is in flight, what waits and why: periodically, and on SIGUSR1."""

    def __init__(self, crawler: Crawler, every: float) -> None:
        self.crawler = crawler
        self.every = every
        self.task: Any = None

    @classmethod
    def from_crawler(cls, crawler: Crawler) -> Self:
        report = cls(crawler, crawler.settings.getfloat("STATUS_EVERY"))
        crawler.signals.connect(report.spider_opened, signal=signals.spider_opened)
        crawler.signals.connect(report.spider_closed, signal=signals.spider_closed)
        return report

    def spider_opened(self, spider: Spider) -> None:
        sigusr1 = getattr(signal, "SIGUSR1", None)
        if sigusr1 is not None:
            try:
                signal.signal(sigusr1, lambda *_: self.report())
            except ValueError:  # not the main thread
                pass
        if self.every > 0:
            self.task = create_looping_call(self.report)
            self.task.start(self.every)

    def spider_closed(self, spider: Spider) -> None:
        if self.task is not None and self.task.running:
            self.task.stop()

    def report(self) -> None:
        engine: Any = self.crawler.engine
        spider = cast(ResumableSpider, self.crawler.spider)
        if engine is None or spider is None:
            return
        settings = self.crawler.settings
        now = time.monotonic()
        downloader: Any = engine.downloader
        in_flight = len(downloader.active)
        scheduler = getattr(getattr(engine, "_slot", None), "scheduler", None)
        waiting = len(scheduler) if scheduler is not None else 0
        store = spider.store
        lines = [
            f"[status] in flight {in_flight}/{settings.getint('CONCURRENT_REQUESTS')},"
            f" waiting in the scheduler {waiting};"
            f" journal: {len(store.in_state(PENDING))} pending, {len(store.in_state(DONE))} done,"
            f" {len(store.in_state(FAILED))} failed; {len(store.book_urls)} books written"
        ]
        for key, slot in downloader.slots.items():
            active, queued = len(slot.active), len(slot.queue)
            for request in list(slot.active)[:5]:
                lines.append(f"[status]   in flight on {key}: {request.url}")
            if queued:
                why = (
                    f"{active} active = CONCURRENT_REQUESTS_PER_DOMAIN {slot.concurrency}"
                    if active >= slot.concurrency
                    else f"DOWNLOAD_DELAY {slot.delay}s, next one in {max(0.0, slot.lastseen + slot.delay - now):.1f}s"
                )
                lines.append(f"[status]   {queued} queued in the downloader for {key}: {why}")
        if waiting and not downloader.slots:
            lines.append("[status]   the scheduler holds requests until robots.txt has been fetched")
        elif waiting:
            lines.append(
                "[status]   scheduler requests wait for a free downloader slot:"
                " the per-domain limit and the download delay pace the crawl"
            )
        if not in_flight and not waiting:
            lines.append("[status]   nothing in flight or waiting")
        print("\n".join(lines), flush=True)


ResumableSpider.custom_settings = {
    **POLITE,
    "FEEDS": {},
    "ITEM_PIPELINES": {DatasetPipeline: 300},
    "EXTENSIONS": {InFlightReport: 500},
}


def nothing_done(message: str) -> int:
    print(f"nothing was done: {message}")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=["crawl", "retry"])
    parser.add_argument("name", help="the name of the crawl; the same name resumes it")
    parser.add_argument("--folder", default="crawls", help="where crawls are kept (default: crawls)")
    parser.add_argument("--kill-after", type=int, default=0, help="SIGKILL this process after N books (a test aid)")
    parser.add_argument("--status-every", type=float, default=10.0, help="seconds between status reports, 0 = off")
    args = parser.parse_args(argv)

    store = CrawlStore(Path(args.folder) / args.name)
    only: set[str] | None = None
    if args.command == "crawl":
        if not store.entries:
            store.discover(ROOT, "start", "")
        if not store.in_state(PENDING):
            failed = len(store.in_state(FAILED))
            store.close()
            return nothing_done(
                f"crawl {args.name!r} is complete: every request found ended, {len(store.book_urls)} books are in the"
                f" dataset and no request is pending."
                + (f" {failed} requests failed; `retry {args.name}` repeats them." if failed else " None failed.")
            )
    else:
        if not store.entries:
            store.close()
            return nothing_done(f"there is no crawl named {args.name!r}, so no request failed.")
        failed_urls = store.in_state(FAILED)
        if not failed_urls:
            pending = len(store.in_state(PENDING))
            store.close()
            return nothing_done(
                f"no request of crawl {args.name!r} is recorded as failed."
                + (f" {pending} requests are still pending; `crawl {args.name}` continues them." if pending else "")
            )
        store.requeue_failed()
        only = set(failed_urls)

    process = CrawlerProcess(
        settings={
            "STATUS_EVERY": args.status_every,
            "KILL_AFTER": args.kill_after,
            "LOG_LEVEL": "INFO",
        }
    )
    process.crawl(ResumableSpider, store=store, only=only)
    process.start()
    store.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
