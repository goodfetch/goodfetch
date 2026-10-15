"""Golden scenario 07: resume a killed crawl by its Run ID, then retry what failed. It imports scenario 05's crawl and
runs only as a script (`python -m scenarios.s07_resume_and_retry [run-id]`).

Run it, kill it part-way, run it again with the same ID: the start resumes the Run from its committed state and the
Dataset (keyed on the book's URL) ends with every book once. After the Run finishes, the requests that ended `failed`
are re-admitted in the same Run. A run of the program that finds the Run already ended does nothing and says why.
"""

import asyncio
import sys

from goodfetch_foxhound import (
    AsyncCrawlRun,
    AsyncFoxhound,
    CrawlRequestBlocked,
    CrawlRequestFailed,
    CrawlRequestOutcome,
    CrawlRequestSkipped,
    CrawlRunProgress,
    CrawlRunReport,
)
from scenarios.s05_two_page_crawl import Book, books, crawl, seeds

# The name of the Run: a start with an ID that names an existing Run resumes it, returns it ended, or refuses.
RUN_ID = sys.argv[1] if len(sys.argv) > 1 else "books-resume-and-retry"


def describe(progress: CrawlRunProgress) -> list[str]:
    """What is in flight and what is waiting, and why."""
    counts = progress.requests
    lines = [
        f"[{progress.lifecycle}] pending {counts.pending}, fetching {counts.fetching}, handling {counts.handling}, "
        f"paused {counts.paused}; done {counts.succeeded} succeeded, {counts.failed} failed, "
        f"{counts.skipped} skipped, {counts.blocked} blocked"
    ]
    for flight in progress.in_flight:
        line = (
            f"  in flight: {flight.request.url} ({flight.request.label}) {flight.phase} "
            f"via {flight.tier}, attempt {flight.attempt}, since {flight.since:%H:%M:%S}"
        )
        if flight.waiting is not None:
            line += f", waiting for a permit: {flight.waiting.reason}"
        lines.append(line)
    for group in progress.waiting:
        line = f"  waiting: {group.requests} requests held by {group.reason}"
        if group.setting is not None:
            line += f" (setting {group.setting.setting} = {group.setting.value})"
        lines.append(line)
        for domain in group.domains:
            until = f"{domain.until:%H:%M:%S}" if domain.until is not None else "until something else happens"
            lines.append(f"    {domain.domain}: {domain.requests} requests, next admitted {until}")
        if group.more_domains:
            lines.append(f"    and {group.more_domains} more domains")
    limit = progress.concurrency.handlers
    lines.append(
        f"  handlers running: {limit.running} (limit {limit.limit.setting} = {limit.limit.value}); "
        f"governor {progress.concurrency.governor.level}"
    )
    return lines


async def watch(run: AsyncCrawlRun) -> None:
    """Prints the Run's progress until the Run ends."""
    async for progress in run.progress():
        for line in describe(progress):
            print(line, flush=True)


async def finish_watching(run: AsyncCrawlRun) -> CrawlRunReport:
    watcher = asyncio.create_task(watch(run))
    try:
        return await run.finish()
    finally:
        watcher.cancel()
        await asyncio.gather(watcher, return_exceptions=True)


def say_nothing_done(report: CrawlRunReport) -> None:
    done = report.nothing_done
    if done is None:
        return
    when = f" (it ended at {done.ended_at:%Y-%m-%d %H:%M:%S})" if done.ended_at is not None else ""
    explanation = {
        "already_ended": "the Run had already ended, so there was nothing to run",
        "nothing_pending": "no request was left pending and no request had failed",
        "no_seed_handled": "no seed reached a handler",
    }.get(done.reason, done.reason)
    print(f"nothing done: {done.reason}: {explanation}{when}", flush=True)


async def print_problems(run: AsyncCrawlRun) -> None:
    # A retried request ends again and its new outcome supersedes the old one: key by request ID.
    latest: dict[str, CrawlRequestOutcome] = {}
    async for outcome in run.outcomes():
        latest[outcome.request.id] = outcome
    for outcome in latest.values():
        match outcome:
            case CrawlRequestFailed(request=request, error=error, attempts=attempts):
                print(f"failed {request.url} after {attempts} attempts: {error.code}: {error}")
            case CrawlRequestSkipped(request=request, reason=reason, message=message):
                print(f"skipped {request.url}: {reason} {message or ''}")
            case CrawlRequestBlocked(request=request, block_cause=cause):
                print(f"blocked {request.url}: {cause}")
            case _:
                pass


async def main() -> None:
    async with (
        AsyncFoxhound.connect() as foxhound,
        await foxhound.start_crawl(crawl, seeds=seeds, id=RUN_ID) as run,
    ):
        print(f"run {RUN_ID}: started as {run.started_as}", flush=True)

        if run.started_as == "ended":
            # Finished by an earlier run of this program: nothing to continue and nothing to retry.
            report = await run.finish()
            say_nothing_done(report)
            if report.nothing_done is None:
                print("nothing done: the Run had already ended", flush=True)
            print(f"{report.requests.succeeded} succeeded, {report.requests.failed} failed")
            return

        # A new Run starts from the seeds; a resumed one continues from its committed state.
        report = await finish_watching(run)
        say_nothing_done(report)

        # Retry only the requests that ended `failed` in this crawl, in the same Run, keeping their IDs.
        retried = 0
        if report.requests.failed > 0:
            retried = await run.retry_failed()
            print(f"retrying {retried} failed requests", flush=True)
            if retried > 0:
                report = await finish_watching(run)
        if retried == 0 and report.nothing_done is None and report.requests.failed == 0:
            print("no request failed, so there was nothing to retry", flush=True)

        # The Dataset is keyed on the book's URL, so a request replayed after the kill leaves each book once.
        urls: list[str] = []
        async for record in run.records(books):
            book: Book = record.value
            urls.append(book.url)
        print(f"{len(urls)} records, {len(set(urls))} distinct books", flush=True)

        exported = await run.export(books, format="json")
        await exported.save_as("books.json")
        print(f"exported {exported.records} books to books.json")

        await print_problems(run)
        totals = report.requests
        print(
            f"{totals.succeeded} succeeded, {totals.failed} failed, {totals.skipped} skipped, "
            f"{totals.blocked} blocked, {totals.pending} pending"
        )
        say_nothing_done(report)


if __name__ == "__main__":
    asyncio.run(main())
