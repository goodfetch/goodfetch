"""Golden scenario 07: the crawl of scenario 05 under an ID of the program's choosing (`python -m
scenarios.s07_resume_and_retry`). Run it, kill it, run it again: the start resumes the Run; once crawled, it retries
only the failed requests; a third run finds the Run ended and says so."""

import asyncio
import time

from goodfetch_foxhound import AsyncCrawlRun, AsyncFoxhound, CrawlRunReport
from scenarios.s05_two_page_crawl import books, crawl, seeds

RUN_ID = "books-toscrape-2026-10-15"


async def print_progress(run: AsyncCrawlRun) -> None:
    """What is in flight and what waits, and why, every five seconds at most."""
    printed_at = 0.0
    async for progress in run.progress():
        if time.monotonic() - printed_at < 5:
            continue
        printed_at = time.monotonic()
        counts = progress.requests
        print(
            f"{counts.pending} pending, {counts.succeeded} succeeded, {counts.failed} failed, {counts.skipped} skipped"
        )
        for flight in progress.in_flight:
            print(
                f"  in flight: {flight.request.url} ({flight.phase} over {flight.tier}, attempt {flight.attempt})"
            )
        for wait in progress.waiting:
            setting = (
                f" ({wait.setting.setting} = {wait.setting.value})"
                if wait.setting
                else ""
            )
            print(f"  waiting: {wait.requests} for {wait.reason}{setting}")


async def crawl_to_end(run: AsyncCrawlRun) -> CrawlRunReport:
    report, _ = await asyncio.gather(run.finish(), print_progress(run))
    return report


async def main() -> None:
    # The same ID begins the Run, resumes it after a kill, or returns it ended. A changed program (seeds, a Label gone, a
    # Payload schema changed) or a damaged run file raises a typed error. Ctrl+C unwinds into `async with`, which stops
    # the Run cleanly: the requests in flight return to pending, within `timeout.close`.
    async with (
        AsyncFoxhound.connect() as foxhound,
        await foxhound.start_crawl(crawl, seeds=seeds, run_id=RUN_ID) as run,
    ):
        print(f"run {run.id}: {run.started_as}")
        report = await crawl_to_end(run)
        if report.lifecycle == "finished" and run.started_as != "ended":
            # Only the failed requests, in this Run and its Dataset.
            readmitted = await run.retry_failed()
            print(f"retrying {readmitted} failed requests")
            if readmitted > 0:
                report = await crawl_to_end(run)
        if report.nothing_done is not None:
            print(f"nothing done: {report.nothing_done.reason}")
        totals = report.requests
        print(
            f"{report.lifecycle}: {totals.succeeded} succeeded, {totals.failed} failed, {totals.skipped} skipped"
        )
        if report.lifecycle == "finished":
            # Each book once: Records commit with their attempt.
            exported = await run.export(books, format="json")
            await exported.save_as("books.json")
            print(f"exported {exported.records} books to books.json")


if __name__ == "__main__":
    asyncio.run(main())
