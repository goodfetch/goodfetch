"""Crawl outcomes, failed_requests and retry_failed on an Attached engine (issue 1495).

Tests `outcomes()` with `kinds`, `labels`, `codes` and `after="now"`, stalled
reader non-interference, `failed_requests()` and `retry_failed()` (with and
without `blocked=True`), and superseding outcomes by `request.id` across both
async and sync APIs.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

import anyio
import pytest
from goodfetch_collie_client.generated.errors import BlockedError
from goodfetch_collie_client.generated.models import Lifecycle
from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncFoxhound,
    Crawl,
    Foxhound,
    seed,
)
from goodfetch_test_support import Product as ToolProduct
from pydantic import BaseModel

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_foxhound import (
        AsyncCrawlContext,
        CrawlContext,
        CrawlRequestOutcome,
    )
    from goodfetch_foxhound.generated.settings import CrawlSettings
    from goodfetch_test_support import ProvingGround


class Note(BaseModel):
    """One recorded URL from an outcomes test."""

    url: str


def site_settings(proving_ground: ProvingGround) -> CrawlSettings:
    """Plain HTTP settings resolving the Proving ground's hosts."""
    resolve = {host: [address] for host, address in proving_ground.resolve.items()}
    return {
        "fetch": {"tier": "http"},
        "network": {"resolve": resolve},
        "egress": {"private_ranges": list(resolve)},
    }


@pytest.fixture
def product() -> ToolProduct:
    """The test runs Foxhound's executable."""
    return ToolProduct.FOXHOUND


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground seed for this test."""
    return 1495


@pytest.mark.covers(
    "story:6-238",
    "story:6-306",
    "story:12-219",
    "operation:run.outcomes",
    "operation:run.failed_requests",
    "operation:run.retry_failed",
)
@pytest.mark.anyio
async def test_async_outcomes_filters_stalled_reader_and_retry_failed(
    env: Mapping[str, str],
    proving_ground: ProvingGround,
) -> None:
    """Async outcomes, filters, after='now', failed_requests and retry_failed."""
    ok_url = proving_ground.url("sitemap-seed.test", "/books/1")
    skip_url = proving_ground.url("sitemap-seed.test", "/private/books/1")
    fail_url = proving_ground.url("sitemap-seed.test", "/books/2")
    block_url = proving_ground.url("sitemap-seed.test", "/books/3")

    fail_hits = 0
    block_hits = 0
    retry_entered = anyio.Event()
    allow_retry_commit = anyio.Event()

    crawl = AsyncCrawl()
    notes = crawl.dataset("notes", schema=Note)
    main_label = crawl.label("main")
    extra_label = crawl.label("extra")

    @crawl.handle(main_label)
    async def handle_main(ctx: AsyncCrawlContext[None]) -> None:
        nonlocal fail_hits
        if ctx.request.url == fail_url:
            if fail_hits == 0:
                fail_hits += 1
                msg = "transient boom"
                raise RuntimeError(msg)
            retry_entered.set()
            await allow_retry_commit.wait()
        await ctx.push(notes, Note(url=ctx.request.url))

    @crawl.handle(extra_label)
    async def handle_extra(ctx: AsyncCrawlContext[None]) -> None:
        nonlocal block_hits
        if block_hits == 0:
            block_hits += 1
            raise BlockedError("blocked by captcha", details={"cause": "captcha"})
        await ctx.push(notes, Note(url=ctx.request.url))

    crawl_settings: CrawlSettings = {
        **site_settings(proving_ground),
        "crawl": {"handler": {"concurrency": 1}, "retry": {"max": 0}},
    }

    foxhound = await AsyncFoxhound.connect(env=env)
    try:
        run = await foxhound.start_crawl(
            crawl,
            run_id="py-outcomes-retry",
            seeds=[
                seed(ok_url, main_label),
                seed(skip_url, extra_label),
                seed(fail_url, main_label),
                seed(block_url, extra_label),
            ],
            settings=crawl_settings,
        )

        # A stalled reader never slows or blocks the crawl.
        stalled = run.outcomes()
        report = await run.finish()
        assert report.lifecycle == Lifecycle.finished
        await stalled.aclose()

        all_outcomes = [o async for o in run.outcomes()]
        assert len(all_outcomes) == 4
        assert all(
            all_outcomes[i - 1].root.position < all_outcomes[i].root.position
            for i in range(1, len(all_outcomes))
        )

        kind_by_url = {o.root.request.url: o.root.kind for o in all_outcomes}
        assert kind_by_url[ok_url] == "succeeded"
        assert kind_by_url[skip_url] == "skipped"
        assert kind_by_url[fail_url] == "failed"
        assert kind_by_url[block_url] == "blocked"

        bad = [o async for o in run.outcomes(kinds=["failed", "blocked"])]
        assert len(bad) == 2

        extra_only = [o async for o in run.outcomes(labels=[extra_label])]
        assert len(extra_only) == 2
        assert all(o.root.request.label == "extra" for o in extra_only)

        handler_failed_only = [o async for o in run.outcomes(codes=["handler_failed"])]
        assert len(handler_failed_only) == 1
        assert handler_failed_only[0].root.request.url == fail_url

        failed_only = await run.failed_requests()
        assert len(failed_only) == 1
        assert failed_only[0].root.request.url == fail_url
        fail_request_id = failed_only[0].root.request.id

        assert len(await run.failed_requests(blocked=True)) == 2

        last_before = all_outcomes[-1].root.position
        assert await run.retry_failed(settings=crawl_settings) == 1
        await retry_entered.wait()
        later: list[CrawlRequestOutcome] = []

        async def watch_later() -> None:
            later.extend([o async for o in run.outcomes(after="now")])

        async with anyio.create_task_group() as tg:
            tg.start_soon(watch_later)
            # Let `watch_later` open `run.outcomes(after="now")` before committing.
            await anyio.lowlevel.checkpoint()
            await run.failed_requests()
            allow_retry_commit.set()
            after_retry = await run.finish()
            assert after_retry.lifecycle == Lifecycle.finished

        assert len(later) == 1
        assert later[0].root.position > last_before
        assert later[0].root.request.id == fail_request_id
        assert later[0].root.kind == "succeeded"
        assert await run.failed_requests() == []
        assert await run.retry_failed(settings=crawl_settings) == 0

        blocked_left = await run.failed_requests(blocked=True)
        assert len(blocked_left) == 1
        assert blocked_left[0].root.request.url == block_url
        assert await run.retry_failed(blocked=True, settings=crawl_settings) == 1
        final_report = await run.finish()
        assert final_report.lifecycle == Lifecycle.finished
        assert await run.failed_requests(blocked=True) == []
        assert await run.retry_failed(blocked=True, settings=crawl_settings) == 0

        full_log = [o async for o in run.outcomes()]
        fail_history = [
            o.root.kind for o in full_log if o.root.request.id == fail_request_id
        ]
        assert fail_history == ["failed", "succeeded"]
    finally:
        await foxhound.close()


@pytest.mark.covers(
    "story:6-306",
    "operation:run.outcomes",
    "operation:run.failed_requests",
    "operation:run.retry_failed",
)
def test_sync_outcomes_failed_requests_and_retry_failed(
    env: Mapping[str, str],
    proving_ground: ProvingGround,
) -> None:
    """Sync CrawlRun.outcomes, failed_requests and retry_failed."""
    fail_url = proving_ground.start_url("catalog/1") + "?sync=1"
    hits = 0

    crawl = Crawl()
    page = crawl.label("page")

    @crawl.handle(page)
    def handle_page(_ctx: CrawlContext[None]) -> None:
        nonlocal hits
        if hits == 0:
            hits += 1
            msg = "first try failed"
            raise RuntimeError(msg)

    sync_settings: CrawlSettings = {
        **site_settings(proving_ground),
        "crawl": {"retry": {"max": 0}},
    }

    with Foxhound.connect(env=env) as foxhound:
        run = foxhound.start_crawl(
            crawl,
            seeds=[seed(fail_url, page)],
            settings=sync_settings,
        )
        assert run.finish().lifecycle == Lifecycle.finished
        assert len(run.failed_requests()) == 1
        assert run.retry_failed(settings=sync_settings) == 1
        assert run.finish().lifecycle == Lifecycle.finished
        assert run.retry_failed(settings=sync_settings) == 0
        assert [o.root.kind for o in run.outcomes()] == ["failed", "succeeded"]
