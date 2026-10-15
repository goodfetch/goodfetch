"""Scenario 07: a crawl under a chosen name that resumes after a kill, then retries only its failed requests.

Each start under RUN_ID says how it opened the Run: it begins the crawl, resumes it from its committed state, or finds it
already ended and runs nothing. A started or resumed Run finishes its crawl, retries the requests that ended `failed` when
there are any, and writes its Dataset out. While the crawl runs, progress lines say what is in flight and what waits, and
why. A start that finds nothing left to do says so, with the reason the engine gives.
"""

import asyncio
import signal
import time
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager

from goodfetch_foxhound import (
    AsyncCrawlRun,
    AsyncFoxhound,
    CrawlRequestBlocked,
    CrawlRequestFailed,
    CrawlRequestOutcome,
    CrawlRequestSkipped,
    CrawlRunProgress,
    CrawlRunReport,
    LimitSource,
    StartedAs,
)

from scenarios.s05_two_page_crawl import books, crawl, seeds

RUN_ID = "s07-books-crawl"
"""The Run's name: a start under it resumes the crawl, whatever state the last start left it in."""
EXPORT_PATH = "books-s07.json"
PROGRESS_INTERVAL = 2.0
"""Seconds between progress lines, so a fast crawl does not flood the terminal."""


def cancel_on_sigterm() -> None:
    """A SIGTERM cancels the main task, so the Run closes with a Checkpoint and a start under its name resumes it."""
    task = asyncio.current_task()
    if task is not None:
        asyncio.get_running_loop().add_signal_handler(signal.SIGTERM, task.cancel)


def describe_start(started_as: StartedAs) -> str:
    if started_as == "new":
        return "starting the crawl"
    if started_as == "resumed":
        return "resuming the crawl from its committed state"
    if started_as == "ended":
        return "the Run had already ended"
    return f"started as {started_as}"


def describe_setting(setting: LimitSource | None) -> str:
    if setting is None:
        return ""
    return f" (setting {setting.setting} = {setting.value}, from the {setting.layer} layer)"


def describe_progress(progress: CrawlRunProgress) -> str:
    counts = progress.requests
    lines = [
        f"progress at {progress.at:%H:%M:%S}: {counts.pending} pending, {counts.fetching} fetching, "
        f"{counts.handling} handling, {counts.succeeded} succeeded, {counts.failed} failed, "
        f"{counts.skipped} skipped, {counts.blocked} blocked"
    ]
    for flight in progress.in_flight:
        held = ""
        if flight.waiting is not None:
            held = f", held by {flight.waiting.reason}{describe_setting(flight.waiting.setting)}"
        lines.append(
            f"  in flight: {flight.phase} {flight.request.url} (tier {flight.tier}, attempt {flight.attempt}{held})"
        )
    for group in progress.waiting:
        lines.append(f"  waiting: {group.requests} requests held by {group.reason}{describe_setting(group.setting)}")
        for domain in group.domains:
            until_at = domain.until
            until = f"until {until_at:%H:%M:%S}" if until_at is not None else "until something else happens"
            lines.append(f"    {domain.domain}: {domain.requests} requests, {until}")
        if group.more_domains:
            lines.append(f"    and {group.more_domains} more domains")
    return "\n".join(lines)


async def print_progress(run: AsyncCrawlRun) -> None:
    last_printed = float("-inf")
    async for progress in run.progress():
        now = time.monotonic()
        if now - last_printed < PROGRESS_INTERVAL:
            continue
        last_printed = now
        print(describe_progress(progress), flush=True)


@asynccontextmanager
async def printing_progress(run: AsyncCrawlRun) -> AsyncIterator[None]:
    printer = asyncio.create_task(print_progress(run))
    try:
        yield
    finally:
        printer.cancel()
        await asyncio.gather(printer, return_exceptions=True)


def describe_nothing_done(started_as: StartedAs, report: CrawlRunReport) -> str | None:
    nothing = report.nothing_done
    if nothing is None:
        if started_as == "ended":
            return f"nothing done: {RUN_ID} had already ended, so starting it again ran no crawl and no retry"
        return None
    if nothing.reason == "already_ended":
        return (
            f"nothing done: {RUN_ID} had already ended at {nothing.ended_at}; a start under its name returns an ended "
            "Run as it is, so no crawl and no retry ran"
        )
    if nothing.reason == "nothing_pending":
        return "nothing done: no request was pending or left to retry, so there was nothing to resume or retry"
    if nothing.reason == "no_seed_handled":
        return (
            f"nothing done: no seed reached a handler (refused {dict(nothing.refused)}, "
            f"skipped {dict(nothing.skipped)})"
        )
    return f"nothing done: the Run reports {nothing.reason}"


async def check_dataset(run: AsyncCrawlRun) -> None:
    urls = [record.value.url async for record in run.records(books)]
    duplicates = len(urls) - len(set(urls))
    verdict = "every book exactly once" if duplicates == 0 else f"{duplicates} books more than once"
    print(f"dataset books: {len(urls)} records, {verdict}", flush=True)


async def report_outcomes(run: AsyncCrawlRun) -> None:
    """Prints each request's latest outcome: a retry's outcome supersedes the failure it retried."""
    latest: dict[str, CrawlRequestOutcome] = {}
    async for outcome in run.outcomes():
        latest[outcome.request.id] = outcome
    for outcome in latest.values():
        match outcome:
            case CrawlRequestFailed(request=request, error=error):
                print(f"still failed {request.url}: {error.code}: {error}")
            case CrawlRequestSkipped(request=request, reason=reason, message=message):
                print(f"skipped {request.url}: {reason} {message or ''}")
            case CrawlRequestBlocked(request=request, block_cause=cause):
                print(f"blocked {request.url}: {cause}")
            case _:
                pass


async def main() -> None:
    cancel_on_sigterm()
    async with (
        AsyncFoxhound.connect() as foxhound,
        await foxhound.start_crawl(crawl, seeds=seeds, id=RUN_ID) as run,
    ):
        started_as = run.started_as
        print(f"run {RUN_ID}: {describe_start(started_as)}", flush=True)
        async with printing_progress(run):
            report = await run.finish()
        if report.requests.failed > 0 and started_as != "ended":
            retried = await run.retry_failed()
            print(
                f"re-admitted {retried} failed requests for retry; skipped and blocked requests are not retried",
                flush=True,
            )
            async with printing_progress(run):
                report = await run.finish()
        exported = await run.export(books, format="json")
        await exported.save_as(EXPORT_PATH)
        print(f"exported {exported.records} books to {EXPORT_PATH}", flush=True)
        await check_dataset(run)
        await report_outcomes(run)
        totals = report.requests
        print(
            f"{totals.succeeded} succeeded, {totals.failed} failed, {totals.skipped} skipped, {totals.blocked} blocked",
            flush=True,
        )
        message = describe_nothing_done(started_as, report)
        if message is not None:
            print(message, flush=True)


if __name__ == "__main__":
    asyncio.run(main())
