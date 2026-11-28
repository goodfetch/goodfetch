"""Multi-process Crawl workers in Python (spec #611's #1497; ADR 0024).

`workers=N` starts `N - 1` `multiprocessing` spawn processes importing the
module-level `AsyncCrawl` or `Crawl`, connecting to the Attached engine's
private endpoint, joining via `crawl.serve`, committing no Record twice,
recovering when one worker dies, and exiting cleanly when the starter closes
the Run.
"""

from __future__ import annotations

import os
import signal
import tempfile
import time
from datetime import timedelta
from pathlib import Path
from typing import TYPE_CHECKING

import anyio
import pytest
from goodfetch_collie_client.generated.errors import SettingInvalidError
from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncCrawlContext,
    AsyncFoxhound,
    Crawl,
    CrawlContext,
    Foxhound,
    seed,
)
from goodfetch_foxhound.generated.errors import CrawlNotImportableError
from goodfetch_test_support import Product as ToolProduct
from pydantic import BaseModel

if TYPE_CHECKING:
    from goodfetch_foxhound.generated.settings import FoxhoundSettings
    from goodfetch_test_support.proving_ground import ProvingGround


@pytest.fixture
def product() -> ToolProduct:
    """The test runs Foxhound's executable."""
    return ToolProduct.FOXHOUND


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground seed."""
    return 1467


class WorkerPayload(BaseModel):
    """Per-seed test controls passed to module-level worker handlers."""

    starter_pid: int | None = None
    kill_marker: str | None = None
    delay_s: float = 0.0


class HandledPage(BaseModel):
    """Record committed by a worker process naming the URL and its PID."""

    url: str
    pid: int


# Module-level AsyncCrawl for `workers > 1` spawn tests.
async_catalog_crawl = AsyncCrawl()
async_listing = async_catalog_crawl.label("listing", payload=WorkerPayload)
async_item = async_catalog_crawl.label("item", payload=WorkerPayload)
async_pages = async_catalog_crawl.dataset("pages", schema=HandledPage)


@async_catalog_crawl.handle(async_listing)
async def _handle_async_listing(ctx: AsyncCrawlContext[WorkerPayload]) -> None:
    payload = ctx.request.payload
    await ctx.enqueue_links(ctx.page.locator("li.product a"), async_item, payload)
    await ctx.enqueue_links(
        ctx.page.locator("a[rel='next'], a.next"), async_listing, payload
    )


@async_catalog_crawl.handle(async_item)
async def _handle_async_item(ctx: AsyncCrawlContext[WorkerPayload]) -> None:
    payload = ctx.request.payload
    if payload.delay_s > 0:
        await anyio.sleep(payload.delay_s)
    if (
        payload.kill_marker is not None
        and payload.starter_pid is not None
        and os.getpid() != payload.starter_pid
    ):
        try:
            fd = os.open(payload.kill_marker, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            os.close(fd)
            should_die = True
        except FileExistsError:
            should_die = False
        if should_die:
            os.kill(os.getpid(), signal.SIGKILL)
    await ctx.push(async_pages, HandledPage(url=ctx.request.url, pid=os.getpid()))


# Module-level sync Crawl for `workers > 1` spawn tests.
sync_catalog_crawl = Crawl()
sync_listing = sync_catalog_crawl.label("listing", payload=WorkerPayload)
sync_item = sync_catalog_crawl.label("item", payload=WorkerPayload)
sync_pages = sync_catalog_crawl.dataset("pages", schema=HandledPage)


@sync_catalog_crawl.handle(sync_listing)
def _handle_sync_listing(ctx: CrawlContext[WorkerPayload]) -> None:
    payload = ctx.request.payload
    ctx.enqueue_links(ctx.page.locator("li.product a"), sync_item, payload)
    ctx.enqueue_links(ctx.page.locator("a[rel='next'], a.next"), sync_listing, payload)


@sync_catalog_crawl.handle(sync_item)
def _handle_sync_item(ctx: CrawlContext[WorkerPayload]) -> None:
    payload = ctx.request.payload
    if payload.delay_s > 0:
        time.sleep(payload.delay_s)
    ctx.push(sync_pages, HandledPage(url=ctx.request.url, pid=os.getpid()))


def _fast_settings(proving_ground: ProvingGround) -> FoxhoundSettings:
    resolve: dict[str, list[str]] = {
        host: [address] for host, address in proving_ground.resolve.items()
    }
    return {
        "fetch": {"tier": "http"},
        "politeness": {"domain": {"concurrency": 16, "delay": timedelta(0)}},
        "network": {"resolve": resolve},
        "egress": {"private_ranges": list(resolve)},
    }


def _pid_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


@pytest.mark.covers(
    "story:3-17",
    "operation:crawl.serve",
    "error:crawl_not_importable",
)
@pytest.mark.anyio
async def test_workers_across_processes_async(
    env: dict[str, str], proving_ground: ProvingGround
) -> None:
    """Covers workers=3 on catalog, crawl_not_importable, kill recovery, close exit."""
    catalog_start = proving_ground.start_url("catalog/1")
    with tempfile.TemporaryDirectory(prefix="gf-py-workers-") as marker_dir:
        async with await AsyncFoxhound.connect(
            env=env, settings=_fast_settings(proving_ground)
        ) as foxhound:
            # 1. Function-local Crawl refused with `crawl_not_importable` + `fix`.
            local_crawl = AsyncCrawl()
            local_page = local_crawl.label("page")

            @local_crawl.handle(local_page)
            async def _local_handler(_ctx: AsyncCrawlContext[None]) -> None:
                return None

            with pytest.raises(CrawlNotImportableError) as exc_info:
                await foxhound.start_crawl(
                    local_crawl,
                    seeds=[seed(catalog_start, local_page)],
                    workers=3,
                )
            assert exc_info.value.retryable is False
            assert exc_info.value.fix

            with pytest.raises(SettingInvalidError):
                await foxhound.start_crawl(
                    async_catalog_crawl,
                    seeds=[seed(catalog_start, async_listing, WorkerPayload())],
                    workers=0,
                )

            # 2. workers=3 on catalog family: 3 distinct PIDs, no duplicate Records.
            async with await foxhound.start_crawl(
                async_catalog_crawl,
                seeds=[seed(catalog_start, async_listing, WorkerPayload(delay_s=0.08))],
                settings={"crawl": {"handler": {"concurrency": 1}}},
                workers=3,
            ) as run:
                report = await run.finish()
                assert report.lifecycle == "finished"
                assert report.completeness == "complete"
                urls: list[str] = []
                pids: set[int] = set()
                async for record in run.records(async_pages):
                    urls.append(record.value.url)
                    pids.add(record.value.pid)
                assert len(urls) >= 6
                assert len(set(urls)) == len(urls)
                assert len(pids) == 3

            # 3. Killing one worker leaves its attempt client_gone/retried
            #    and the crawl completes with no duplicate Record.
            kill_marker = str(Path(marker_dir) / "killed-once")
            async with await foxhound.start_crawl(
                async_catalog_crawl,
                seeds=[
                    seed(
                        catalog_start,
                        async_listing,
                        WorkerPayload(
                            starter_pid=os.getpid(),
                            kill_marker=kill_marker,
                            delay_s=0.08,
                        ),
                    )
                ],
                settings={
                    "crawl": {"handler": {"concurrency": 1}, "retry": {"max": 2}}
                },
                workers=3,
            ) as recovery_run:
                report = await recovery_run.finish()
                assert report.lifecycle == "finished"
                assert report.completeness == "complete"
                committed_urls = [
                    record.value.url
                    async for record in recovery_run.records(async_pages)
                ]
                assert len(committed_urls) >= 6
                assert len(set(committed_urls)) == len(committed_urls)

                saw_client_gone_retry = False
                async for outcome in recovery_run.outcomes():
                    inner = outcome.root
                    if inner.attempts > 1 and any(
                        error.code == "client_gone" for error in inner.errors
                    ):
                        saw_client_gone_retry = True
                assert saw_client_gone_retry is True

            # 4. Workers exit when the starter closes an active keep_alive Run.
            keep_run = await foxhound.start_crawl(
                async_catalog_crawl,
                seeds=[seed(catalog_start, async_listing, WorkerPayload(delay_s=0.08))],
                settings={"crawl": {"handler": {"concurrency": 1}, "keep_alive": True}},
                workers=3,
            )
            worker_pids: set[int] = set()
            async with keep_run.records(async_pages) as records:
                async for record in records:
                    if record.value.pid != os.getpid():
                        worker_pids.add(record.value.pid)
                    if len(worker_pids) >= 2:
                        break
            assert len(worker_pids) == 2
            await keep_run.close()
            for pid in worker_pids:
                assert not _pid_alive(pid), f"worker pid {pid} still alive after close"


@pytest.mark.covers(
    "story:3-17",
    "operation:crawl.serve",
    "error:crawl_not_importable",
)
def test_workers_across_processes_sync(
    env: dict[str, str], proving_ground: ProvingGround
) -> None:
    """Sync `Foxhound.start_crawl(..., workers=3)` runs across 3 processes."""
    catalog_start = proving_ground.start_url("catalog/1")
    with Foxhound.connect(env=env, settings=_fast_settings(proving_ground)) as foxhound:
        local_crawl = Crawl()
        local_page = local_crawl.label("page")

        @local_crawl.handle(local_page)
        def _local_sync(_ctx: CrawlContext[None]) -> None:
            return None

        with pytest.raises(CrawlNotImportableError) as exc_info:
            foxhound.start_crawl(
                local_crawl,
                seeds=[seed(catalog_start, local_page)],
                workers=3,
            )
        assert exc_info.value.fix

        with foxhound.start_crawl(
            sync_catalog_crawl,
            seeds=[seed(catalog_start, sync_listing, WorkerPayload(delay_s=0.08))],
            settings={"crawl": {"handler": {"concurrency": 1}}},
            workers=3,
        ) as run:
            report = run.finish()
            assert report.lifecycle == "finished"
            assert report.completeness == "complete"
            urls: list[str] = []
            pids: set[int] = set()
            for record in run.records(sync_pages):
                urls.append(record.value.url)
                pids.add(record.value.pid)
            assert len(urls) >= 6
            assert len(set(urls)) == len(urls)
            assert len(pids) == 3
