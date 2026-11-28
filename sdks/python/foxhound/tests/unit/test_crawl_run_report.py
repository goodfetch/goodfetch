"""A crawl's own report and progress (ADR 0120) on the Scripted engine (ADR 0069).

`start_crawl` sends the crawl's Run ID, tags and Limits, and its Run's
`progress()`, `stop()`, `report()` and `finish()` answer Foxhound's `CrawlRunProgress`
and `CrawlRunReport`, the crawl's own parts beside every Run's, its
instants as UTC datetimes. TypeScript's `crawl-run.test.ts` of
`@goodfetch/foxhound` is its twin.
"""

from __future__ import annotations

from datetime import UTC, datetime
from typing import TYPE_CHECKING

import pytest
from goodfetch_collie_client.limits import Limits
from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncCrawlContext,
    Crawl,
    CrawlContext,
    seed,
)
from goodfetch_foxhound.testing import async_connect_over, connect_over
from goodfetch_test_support import Finished, play_script

if TYPE_CHECKING:
    from goodfetch_foxhound.generated.models import CrawlRunProgress, CrawlRunReport

SCRIPT = "sdk-python/crawl/report-and-progress.json"
"""The crawl whose progress waits idle, then stopped and reported."""
SEED = "https://shop.test/catalog/1"
"""The crawl's one seed, a listing page."""
STOPPED_AT = datetime(2026, 1, 1, 0, 0, 5, tzinfo=UTC)
"""When the script's crawl stopped, and its Checkpoint was written."""


def _expect_idle(progress: CrawlRunProgress | None) -> None:
    """The progress the script answers: idle, a request in flight, by state."""
    assert progress is not None
    assert progress.requests is not None
    assert progress.in_flight is not None
    assert progress.concurrency is not None
    assert progress.requests.by_skip_reason is not None
    assert (
        progress.requests.in_human_pause,
        progress.requests.succeeded,
        [(group.name, group.count) for group in progress.requests.by_skip_reason],
        [
            (flight.request_id, flight.phase, flight.started_at)
            for flight in progress.in_flight
        ],
        progress.concurrency.handlers,
        progress.concurrency.handler_concurrency,
    ) == (
        0,
        1,
        [("max_requests", 1)],
        [("1", "handling", datetime(2026, 1, 1, 0, 0, 1, tzinfo=UTC))],
        1,
        8,
    )


def _expect_stopped(report: CrawlRunReport) -> None:
    """The report the script answers: stopped, `incomplete` by a cap, totals."""
    assert report.stop_reason is not None
    assert report.completeness_reasons is not None
    assert report.checkpoint is not None
    assert report.requests is not None
    assert report.records is not None
    assert (
        report.lifecycle,
        report.stop_reason.model_dump(exclude_none=True),
        report.completeness,
        [(reason.code, reason.count) for reason in report.completeness_reasons],
        report.ended_at,
        (report.checkpoint.journal_position, report.checkpoint.time),
        (report.requests.succeeded, report.requests.skipped),
        [
            (dataset.dataset, dataset.committed, dataset.dropped, dataset.rejected)
            for dataset in report.records
        ],
        report.learned_rules,
    ) == (
        "stopped",
        {"kind": "abort", "by": "program"},
        "incomplete",
        [("crawl.max_requests", 1)],
        STOPPED_AT,
        (7, STOPPED_AT),
        (1, 1),
        [("products", 1, 0, 0)],
        [],
    )


@pytest.mark.covers("story:11-1080", "story:11-1081", "story:11-1082")
@pytest.mark.anyio
async def test_reads_its_progress_until_idle_then_stops_and_reports_a_crawls() -> None:
    """Its progress, its stop, its report and its finish read as a crawl's own."""
    crawl = AsyncCrawl()
    listing = crawl.label("listing")
    product = crawl.label("product")
    crawl.dataset("products", key=["id"])

    # The script leases no work item: each handler only has to exist.
    @crawl.handle(listing)
    async def read_listing(_: AsyncCrawlContext[None]) -> None:
        return

    @crawl.handle(product)
    async def read_product(_: AsyncCrawlContext[None]) -> None:
        return

    played = play_script(SCRIPT)
    foxhound = await async_connect_over(played.transport)

    run = await foxhound.start_crawl(
        crawl,
        seeds=[seed(SEED, listing)],
        run_id="kept",
        tags=["crawls"],
        limits=Limits(requests=10),
    )
    idle: CrawlRunProgress | None = None
    async with run.progress() as progress:
        async for value in progress:
            if value.idle:
                idle = value
                break
    stopped = await run.stop()
    report = await run.report()
    finished = await run.finish()
    await foxhound.close()

    _expect_idle(idle)
    _expect_stopped(stopped)
    assert report == stopped
    assert finished == stopped
    assert played.done() == Finished()


@pytest.mark.covers("story:11-1080", "story:11-1081", "story:11-1082")
def test_a_sync_crawls_run_reads_the_same_progress_stop_and_report() -> None:
    """The sync twin, its progress read with `for`."""
    crawl = Crawl()
    listing = crawl.label("listing")
    product = crawl.label("product")
    crawl.dataset("products", key=["id"])

    @crawl.handle(listing)
    def read_listing(_: CrawlContext[None]) -> None:
        return

    @crawl.handle(product)
    def read_product(_: CrawlContext[None]) -> None:
        return

    played = play_script(SCRIPT)

    with connect_over(played.transport) as foxhound:
        run = foxhound.start_crawl(
            crawl,
            seeds=[seed(SEED, listing)],
            run_id="kept",
            tags=["crawls"],
            limits=Limits(requests=10),
        )
        idle: CrawlRunProgress | None = None
        with run.progress() as progress:
            for value in progress:
                if value.idle:
                    idle = value
                    break
        stopped = run.stop()
        report = run.report()
        finished = run.finish()

    _expect_idle(idle)
    _expect_stopped(stopped)
    assert report == stopped
    assert finished == stopped
    assert played.done() == Finished()
