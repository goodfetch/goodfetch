"""Snippet for The Run in code (#478, ADR 0102), on golden scenario 07 with scenario 05's crawl."""

import logging
from collections.abc import AsyncIterable
from datetime import timedelta

from pydantic import BaseModel

from goodfetch import (
    AsyncCrawl,
    AsyncCrawlRun,
    AsyncGoodfetch,
    AsyncRun,
    CrawlRunReport,
    EnqueueNotAdded,
    Goodfetch,
    seed,
)

log = logging.getLogger(__name__)


class BookRecord(BaseModel):
    title: str
    price: str

# Scenario 05's declaration (snippet_crawl.py) has Labels `home`, `category` and `book`, one handler each.
books = AsyncCrawl()
home = books.label("home")
book_records = books.dataset("books", schema=BookRecord)


async def scenario_07(retry_failed: bool) -> CrawlRunReport:
    """One program, run again under the same ID after a kill, then with `retry_failed`, then once more with nothing
    left to do."""
    async with AsyncGoodfetch.connect() as gf:
        run = await gf.foxhound.start_crawl(
            books,
            seeds=[seed("https://books.toscrape.com/", home)],
            id="books-2026-10-15",  # a start under this ID resumes the Run where it stopped
        )
        async with run:
            if run.started_as == "resumed":
                log.info("resuming %s", run.id)
            for answer in run.seeds:
                if isinstance(answer, EnqueueNotAdded):
                    log.warning("seed not added: %s (%s)", answer.url, answer.reason)
            if retry_failed:
                readmitted = await run.retry_failed()  # only `failed`, in this same Run and its Dataset
                log.info("retrying %d requests", readmitted)
            report = await run.finish()  # lets the crawl finish; at once if it had already ended
            if report.nothing_done is not None:
                log.warning("nothing done: %s", report.nothing_done.reason)
            return report


async def follow(run: AsyncCrawlRun) -> None:
    """While it runs: what is in flight, what waits, and why."""
    async for progress in run.progress():
        handlers = progress.concurrency.handlers
        log.info(
            "pending %d, in flight %d, handlers %d of %s",
            progress.requests.pending,
            len(progress.in_flight),
            handlers.running,
            handlers.limit.value,
        )
        for wait in progress.waiting:
            log.info("%d waiting for %s (%s)", wait.requests, wait.reason, wait.setting.setting if wait.setting else "-")


async def now(run: AsyncRun) -> None:
    """One value: leave the loop."""
    async for progress in run.progress():
        log.info("%s, %d pauses waiting", progress.lifecycle, len(progress.pauses))
        break


async def failed_urls(run: AsyncCrawlRun) -> list[str]:
    """The failed URLs, from the Run's committed state, also once it has ended."""
    return [outcome.request.url async for outcome in run.outcomes(kinds=["failed"])]


async def export_yesterday(gf: AsyncGoodfetch) -> None:
    """Yesterday's crawl, read by its ID from another program: nothing resumed, nothing driven."""
    async with await gf.foxhound.open_crawl_run("books-2026-10-14") as view:
        report = await view.report()  # at once: it has ended
        books_totals = report.records.get("books")
        log.info("books committed: %s, sinks: %s", books_totals.committed if books_totals else 0, report.sinks)
        csv = await view.export(book_records, format="csv")
        await csv.save_as("books.csv")


async def place_order(order_id: str) -> None:
    """A Collie Run under an ID: run again after a crash, it finds its Sessions and its Effect journal, so an order placed
    before the crash is checked rather than placed again; once finished, it runs nothing."""
    async with AsyncGoodfetch.connect() as gf, await gf.collie.start(id=f"order-{order_id}") as run:
        if run.started_as == "ended":
            return
        await run.set_progress_message(f"order {order_id}")
        page = await run.new_page()
        await page.goto("https://shop.example/cart")
        await page.act("place the order")
        report = await run.finish()  # `finished` and `complete`; the Verdict comes from its checks
        log.info("ordered: %s, %s, weakened %s", report.verdict, report.usage.price, [w.setting for w in report.weakened])


async def fed(urls: AsyncIterable[str]) -> None:
    """A keep-alive crawl fed while it runs, finished by the program (ADR 0037's RT-12)."""
    async with AsyncGoodfetch.connect() as gf:
        run = await gf.foxhound.start_crawl(
            books,
            seeds=[seed("https://books.toscrape.com/", home)],
            settings={"crawl": {"keep_alive": True}, "checkpoint": {"interval": timedelta(minutes=10)}},
        )
        async with run:
            async for url in urls:
                await run.add_seed(url, home)
            checkpoint = await run.checkpoint(path="/mnt/backup/books.checkpoint")
            log.info("checkpoint %s, %d bytes", checkpoint.digest, checkpoint.size)
            await run.finish()  # ends once what is pending is done


def scenario_07_sync() -> None:
    """The sync client, as in a notebook cell."""
    with Goodfetch.connect() as gf, gf.collie.start(id="notebook-2026-10-15") as run:
        report = run.finish()
        log.info("%s %s %s", report.lifecycle, report.verdict, report.completeness)


async def mistakes(gf: AsyncGoodfetch, run: AsyncCrawlRun) -> None:
    """Typed mistakes, caught by the type checkers before anything runs."""
    await gf.foxhound.start_crawl(books, id="books-2026-10-15")  # type: ignore[call-arg]  # pyright: ignore[reportCallIssue]
    await gf.collie.start(detached=True)  # type: ignore[call-arg]  # pyright: ignore[reportCallIssue]
    await run.retry_failed(skipped=True)  # type: ignore[call-arg]  # pyright: ignore[reportCallIssue]
