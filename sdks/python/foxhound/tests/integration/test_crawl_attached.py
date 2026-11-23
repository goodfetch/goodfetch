"""A program's crawl on Foxhound's executable (ADR 0093; issue 1468).

The catalog program of `foxhound_catalog.py` crawls the Proving ground's
catalog family on an Attached engine of Foxhound's executable of the same
commit, three ways: an `AsyncCrawl` on `AsyncFoxhound` under asyncio and
Trio, and a `Crawl` on the sync `Foxhound`, whose handler runs on the
Connection's thread pool. Each way's Records go to the Proving ground's
oracle, which judges them against the task, so every way gives the same
Records and the same finished report. TypeScript's `crawl.test.ts` of
Foxhound's package is its twin.
"""

from __future__ import annotations

import functools
from dataclasses import dataclass
from typing import TYPE_CHECKING

import anyio.to_thread
import pytest
from foxhound_catalog import AsyncCatalog, Catalog
from goodfetch_foxhound import AsyncFoxhound, Foxhound, seed
from goodfetch_test_support import Product as ToolProduct
from goodfetch_test_support import Way, judge_task, three_ways

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_collie_client.generated.models import RunReport
    from goodfetch_foxhound.generated.settings import CrawlSettings
    from goodfetch_test_support import ClientCaseRecord, ProvingGround

TASK = "catalog/1"
"""The Proving ground's task the crawl starts at, the catalog's first listing."""

SETTINGS: CrawlSettings = {
    "fetch": {"tier": "http"},
    "crawl": {"handler": {"concurrency": 1}},
}
"""The Run's settings: plain HTTP fetches, and one handler at a time."""


@pytest.fixture
def product() -> ToolProduct:
    """The test runs Foxhound's executable."""
    return ToolProduct.FOXHOUND


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground's own seed for these tests, TypeScript's."""
    return 1467


@pytest.fixture
def with_browser() -> bool:
    """The engine serves each attempt a page, on the test's Browser cache."""
    return True


@dataclass(frozen=True)
class Crawled:
    """What a way's crawl gave: its Run's report and its Records' values."""

    report: RunReport
    records: list[object]


async def _crawl_async(env: Mapping[str, str], start: str) -> Crawled:
    """Crawls the catalog from `start` with `AsyncCrawl` on `AsyncFoxhound`."""
    catalog = AsyncCatalog()
    foxhound = await AsyncFoxhound.connect(env=env)
    run = await foxhound.start_crawl(
        catalog.crawl, seeds=[seed(start, catalog.listing)], settings=SETTINGS
    )
    report = await run.finish()
    async with run.records(catalog.products) as read:
        records: list[object] = [record.value async for record in read]
    await foxhound.close()
    return Crawled(report=report, records=records)


def _crawl_sync(env: Mapping[str, str], start: str) -> Crawled:
    """Crawls the catalog from `start` with `Crawl` on the sync `Foxhound`."""
    catalog = Catalog()
    with Foxhound.connect(env=env) as foxhound:
        run = foxhound.start_crawl(
            catalog.crawl, seeds=[seed(start, catalog.listing)], settings=SETTINGS
        )
        report = run.finish()
        with run.records(catalog.products) as read:
            records: list[object] = [record.value for record in read]
    return Crawled(report=report, records=records)


@pytest.mark.covers("story:11-1013", "story:11-1015")
@pytest.mark.client_case(id="crawl-catalog-program", mode="attached")
@three_ways
async def test_crawls_the_catalog_family_and_the_oracle_accepts_its_records(
    way: Way,
    env: Mapping[str, str],
    proving_ground: ProvingGround,
    client_case: ClientCaseRecord,
) -> None:
    """One Label, one handler, one keyed Dataset and one seed, each way."""
    start = await anyio.to_thread.run_sync(proving_ground.start_url, TASK)

    crawled = (
        await anyio.to_thread.run_sync(_crawl_sync, env, start)
        if way is Way.SYNC
        else await _crawl_async(env, start)
    )
    answer = await anyio.to_thread.run_sync(
        functools.partial(judge_task, proving_ground, TASK, crawled.records)
    )
    client_case.oracle_answer(answer)

    with client_case.check("finish() returns the finished Run's report"):
        assert (crawled.report.lifecycle, crawled.report.verdict) == (
            "finished",
            "verified",
        )
    with client_case.check("the oracle accepts the Records"):
        assert answer.outcome == "verified"
