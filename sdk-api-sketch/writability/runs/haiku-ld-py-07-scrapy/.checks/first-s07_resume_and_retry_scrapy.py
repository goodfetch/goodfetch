"""Scenario 07 on Scrapy 2.19.0: a named, resumable crawl of books.toscrape.com with a retry of the failed requests.

    python s07_resume_and_retry_scrapy.py NAME                 start NAME, or resume it where it stopped
    python s07_resume_and_retry_scrapy.py NAME --retry-failed  crawl again only the requests that failed in NAME

Each run names a job. The job's folder jobs/NAME/ holds:
    jobdir/          Scrapy's scheduler state, so a killed crawl resumes (JOBDIR)
    dataset.jsonl    one line per book, each URL once, appended as the books arrive
    failures.json    the requests that failed and are waiting for a retry
    in_flight.json   the requests that had reached the downloader when the process stopped
    status.json      present once the crawl has finished on its own

Whenever a run has nothing to do, it says so and why, and does not start Scrapy.
"""

import argparse
import json
import os
import re
from collections.abc import AsyncIterator, Callable
from datetime import datetime
from pathlib import Path
from typing import Any, Self, TextIO

from scrapy import Request, Spider, signals
from scrapy.crawler import Crawler, CrawlerProcess
from scrapy.exceptions import DropItem, IgnoreRequest
from scrapy.http import Response
from scrapy.spidermiddlewares.httperror import HttpError
from twisted.internet.task import LoopingCall
from twisted.python.failure import Failure

from s05_two_page_crawl_scrapy import BooksSpider

JOBS_ROOT = Path(__file__).resolve().parent / "jobs"
NAME_PATTERN = re.compile(r"[A-Za-z0-9_-]{1,64}")
STATUS_EVERY_SECONDS = 10.0

Json = dict[str, Any]


def _read_json(path: Path, default: Any) -> Any:
    if not path.exists():
        return default
    return json.loads(path.read_text(encoding="utf8"))


def _write_json(path: Path, data: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(data, indent=1, sort_keys=True), encoding="utf8")
    os.replace(tmp, path)  # a kill leaves either the old file or the new one


class JobState:
    """The files of one named job."""

    def __init__(self, name: str) -> None:
        self.name = name
        self.root = JOBS_ROOT / name
        self.jobdir = self.root / "jobdir"
        self.dataset = self.root / "dataset.jsonl"
        self._failures_file = self.root / "failures.json"
        self._in_flight_file = self.root / "in_flight.json"
        self._status_file = self.root / "status.json"

    def load_failures(self) -> dict[str, Json]:
        return _read_json(self._failures_file, {})

    def save_failures(self, failures: dict[str, Json]) -> None:
        _write_json(self._failures_file, failures)

    def load_in_flight(self) -> list[Json]:
        return _read_json(self._in_flight_file, [])

    def save_in_flight(self, entries: list[Json]) -> None:
        _write_json(self._in_flight_file, entries)

    def finished_at(self) -> str | None:
        return _read_json(self._status_file, {}).get("finished_at")

    def mark_finished(self) -> None:
        stamp = datetime.now().isoformat(timespec="seconds")
        _write_json(self._status_file, {"finished_at": stamp})

    def dataset_size(self) -> int:
        if not self.dataset.exists():
            return 0
        with self.dataset.open(encoding="utf8") as lines:
            return sum(1 for _ in lines)


class DatasetPipeline:
    """Appends each book to the job's dataset once; a URL already there is dropped."""

    def __init__(self, path: Path) -> None:
        self.path = path
        self.seen: set[str] = set()
        self.file: TextIO | None = None

    @classmethod
    def from_crawler(cls, crawler: Crawler) -> Self:
        return cls(Path(crawler.settings["DATASET_PATH"]))

    def open_spider(self, spider: Spider) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        text = self.path.read_text(encoding="utf8") if self.path.exists() else ""
        for line in text.splitlines():
            try:
                self.seen.add(json.loads(line)["url"])
            except (ValueError, KeyError, TypeError):
                continue  # a line cut short when a crawl was killed
        self.file = self.path.open("a", encoding="utf8")
        if text and not text.endswith("\n"):
            self.file.write("\n")  # keep the cut-short line apart from the next book

    def process_item(self, item: Any, spider: Spider) -> Any:
        if self.file is None:
            raise RuntimeError("the dataset is not open")
        url = item["url"]
        if url in self.seen:
            raise DropItem(f"already in the dataset: {url}")
        self.seen.add(url)
        self.file.write(json.dumps(item, default=str, ensure_ascii=False) + "\n")
        self.file.flush()
        return item

    def close_spider(self, spider: Spider) -> None:
        if self.file is not None:
            self.file.close()


def _journal_entry(request: Request) -> Json:
    callback = getattr(request.callback, "__name__", "parse")
    return {"url": request.url, "callback": callback, "cb_kwargs": dict(request.cb_kwargs)}


class ResumableBooksSpider(BooksSpider):
    """The books crawl with its state kept in a job, and a mode that retries only the failed requests."""

    custom_settings = {
        **BooksSpider.custom_settings,
        # The dataset is the pipeline's file: a feed export would overwrite it on every run.
        "FEEDS": {},
        "ITEM_PIPELINES": {DatasetPipeline: 300},
    }

    def __init__(self, job: str, retry: bool = False, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self.job = JobState(job)
        self.retrying = retry
        self.ledger: dict[str, Json] = self.job.load_failures()  # failed requests, by URL
        self._failed_now: set[str] = set()  # URLs that failed during this run
        self._in_flight: dict[Request, Json] = {}  # requests that reached the downloader
        self._status_loop = LoopingCall(self._print_status)

    @classmethod
    def from_crawler(cls, crawler: Crawler, *args: Any, **kwargs: Any) -> Self:
        spider = super().from_crawler(crawler, *args, **kwargs)
        crawler.signals.connect(spider._on_opened, signal=signals.spider_opened)
        crawler.signals.connect(
            spider._on_reached_downloader, signal=signals.request_reached_downloader
        )
        crawler.signals.connect(
            spider._on_left_downloader, signal=signals.request_left_downloader
        )
        crawler.signals.connect(
            spider._on_response_received, signal=signals.response_received
        )
        return spider

    async def start(self) -> AsyncIterator[Any]:
        if self.retrying:
            for entry in list(self.ledger.values()):
                yield self._request_from_entry(entry)
            return
        # Requests that were in flight when the last run stopped are not in the
        # scheduler's saved queue, so they are sent again here.
        for entry in self.job.load_in_flight():
            request = self._request_from_entry(entry)
            self._in_flight[request] = entry
            yield request
        async for request in super().start():
            yield request

    def errback(self, failure: Failure) -> None:
        super().errback(failure)
        if failure.check(IgnoreRequest):
            return  # robots.txt and offsite requests are skipped, not failed
        request: Request = failure.request  # type: ignore[attr-defined]
        error = failure.value
        reason = f"HTTP {error.response.status}" if isinstance(error, HttpError) else repr(error)
        self._failed_now.add(request.url)
        self.ledger[request.url] = {**_journal_entry(request), "reason": reason}
        self.job.save_failures(self.ledger)

    def closed(self, reason: str) -> None:
        if self._status_loop.running:
            self._status_loop.stop()
        if reason == "finished" and not self.retrying:
            self.job.mark_finished()
        super().closed(reason)

    def _on_opened(self) -> None:
        self._status_loop.start(STATUS_EVERY_SECONDS, now=True)

    def _on_reached_downloader(self, request: Request) -> None:
        if self.retrying:
            return  # a retry is driven by the failures file alone
        self._in_flight[request] = _journal_entry(request)
        self.job.save_in_flight(list(self._in_flight.values()))

    def _on_left_downloader(self, request: Request) -> None:
        if self._in_flight.pop(request, None) is not None:
            self.job.save_in_flight(list(self._in_flight.values()))

    def _on_response_received(self, response: Response, request: Request) -> None:
        if request.url in self._failed_now:
            return  # a failure of this run is recorded by errback, after the response
        if self.ledger.pop(request.url, None) is not None:
            self.job.save_failures(self.ledger)

    def _callback_named(self, name: str) -> Callable[..., Any]:
        callbacks: dict[str, Callable[..., Any]] = {
            "parse": self.parse,
            "parse_category": self.parse_category,
            "parse_book": self.parse_book,
        }
        return callbacks[name]

    def _request_from_entry(self, entry: Json) -> Request:
        return Request(
            entry["url"],
            callback=self._callback_named(entry["callback"]),
            cb_kwargs=entry["cb_kwargs"],
            errback=self.errback,
            dont_filter=True,  # a request that failed or was in flight is sent again
        )

    def _print_status(self) -> None:
        engine = self.crawler.engine
        downloader = engine.downloader
        stamp = datetime.now().strftime("%H:%M:%S")
        print(
            f"[{stamp}] job {self.job.name!r}: {len(self._in_flight)} requests in flight, "
            f"{len(self.ledger)} failed so far"
        )
        for key, slot in downloader.slots.items():
            for request in slot.transferring:
                print(f"  downloading {request.url}")
            if not slot.queue:
                continue
            if slot.latercall is not None:
                why = f"DOWNLOAD_DELAY of {slot.delay:g}s between requests to {key} has not run out"
            elif len(slot.transferring) >= slot.concurrency:
                why = (
                    f"all {slot.concurrency} connections to {key} are busy "
                    "(CONCURRENT_REQUESTS_PER_DOMAIN)"
                )
            else:
                why = f"the downloader has not taken them up yet for {key}"
            print(f"  waiting {len(slot.queue)} for {key}: {why}")
        scheduler = engine.scheduler
        if scheduler is not None and scheduler.has_pending_requests():
            if downloader.needs_backout():
                why = f"all {downloader.total_concurrency} CONCURRENT_REQUESTS slots are busy"
            else:
                why = "the engine takes the next one up when the downloader has room"
            print(f"  waiting in the scheduler: {why}")


def crawler_settings(job: JobState, *, resumable: bool) -> dict[str, Any]:
    settings: dict[str, Any] = {"DATASET_PATH": str(job.dataset), "LOG_LEVEL": "INFO"}
    if resumable:
        settings["JOBDIR"] = str(job.jobdir)
    return settings


def run_crawl(job: JobState) -> None:
    finished = job.finished_at()
    if finished is not None:
        failed = job.load_failures()
        if failed:
            print(
                f"Nothing done: job {job.name!r} finished its crawl at {finished}, and {len(failed)} "
                "requests failed in it. Run with --retry-failed to crawl only those."
            )
        else:
            print(
                f"Nothing done: job {job.name!r} finished its crawl at {finished} and no request "
                "failed, so no request is left to crawl."
            )
        return
    if job.jobdir.exists():
        print(
            f"Resuming job {job.name!r}: {job.dataset_size()} books are already in the dataset, and "
            f"{len(job.load_in_flight())} requests that were in flight when it stopped are sent again."
        )
    else:
        print(f"Starting job {job.name!r}.")
    process = CrawlerProcess(crawler_settings(job, resumable=True))
    process.crawl(ResumableBooksSpider, job=job.name)
    process.start()


def run_retry(job: JobState) -> None:
    failed = job.load_failures()
    if not failed:
        finished = job.finished_at()
        if finished is None:
            reason = "the crawl has not finished yet and no request has failed so far"
        else:
            reason = f"the crawl finished at {finished} and no request failed in it"
        print(f"Nothing done: job {job.name!r} has no failed request to retry, because {reason}.")
        return
    print(f"Retrying the {len(failed)} failed requests of job {job.name!r}; no other request is sent.")
    process = CrawlerProcess(crawler_settings(job, resumable=False))
    process.crawl(ResumableBooksSpider, job=job.name, retry=True)
    process.start()


def main() -> None:
    parser = argparse.ArgumentParser(description="Crawl books.toscrape.com as a named, resumable job.")
    parser.add_argument(
        "name", help="the job name; running the same name again resumes or retries that job"
    )
    parser.add_argument(
        "--retry-failed",
        action="store_true",
        help="crawl again only the requests that failed in the job",
    )
    args = parser.parse_args()
    if not NAME_PATTERN.fullmatch(args.name):
        parser.error("the job name uses only letters, digits, '-' and '_' (at most 64 characters)")
    job = JobState(args.name)
    if args.retry_failed:
        run_retry(job)
    else:
        run_crawl(job)


if __name__ == "__main__":
    main()
