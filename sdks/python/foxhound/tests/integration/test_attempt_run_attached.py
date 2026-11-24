"""A handler's `ctx.run` on an Attached engine (ADRs 0093 and 0103; issue 1486).

On an Attached engine of Foxhound's executable of the same commit, crawling
the Proving ground's `catalog` family, `ctx.run` reaches the crawl's
Key-value store bound to the attempt, so a write through it commits with
the attempt or is discarded with it, while a write through the program's
own Run value, captured by the handler, commits at once, on every attempt
that runs it. Three ways: an `AsyncCrawl` under asyncio and Trio, and a
`Crawl` on the sync `Foxhound`. TypeScript's `attempt-run.test.ts` of
Foxhound's package is its twin.
"""

from __future__ import annotations

import threading
from dataclasses import dataclass, field
from typing import TYPE_CHECKING

import anyio
import anyio.to_thread
import pytest
from goodfetch_foxhound import AsyncCrawl, AsyncFoxhound, Crawl, Foxhound, seed
from goodfetch_test_support import Product as ToolProduct
from goodfetch_test_support import Way, three_ways

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_foxhound import (
        AsyncCrawlContext,
        AsyncCrawlRun,
        CrawlContext,
        CrawlRun,
    )
    from goodfetch_foxhound.generated.settings import CrawlSettings
    from goodfetch_test_support import ProvingGround

TASK = "catalog/1"
"""The Proving ground's task the crawl starts at, the catalog's first listing."""

SETTINGS: CrawlSettings = {
    "fetch": {"tier": "http"},
    "crawl": {"handler": {"concurrency": 1}, "retry": {"handler_failed": True}},
}
"""Plain HTTP fetches, one handler at a time, and a failed handler retried."""

PNG = "image/png"
"""The content type of the bytes each attempt stores."""


@pytest.fixture
def product() -> ToolProduct:
    """The test runs Foxhound's executable."""
    return ToolProduct.FOXHOUND


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground's own seed for this test, TypeScript's."""
    return 1486


@pytest.fixture
def with_browser() -> bool:
    """The engine serves each attempt a page, on the test's Browser cache."""
    return True


@dataclass
class Seen:
    """What a way's crawl saw, and what its Run had committed once it finished."""

    run_id: str = ""
    """The Run's ID."""
    run_ids: list[str] = field(default_factory=list[str])
    """The ID each attempt's `ctx.run` gave."""
    attempts: list[dict[str, object]] = field(default_factory=list[dict[str, object]])
    """What each attempt saw through its own Run and through the captured one."""
    committed: list[object] = field(default_factory=list[object])
    """What the program's Run held once the crawl finished."""


class FailedAfterItsWritesError(Exception):
    """The first attempt fails after its writes."""


async def _crawl_async(env: Mapping[str, str], start: str) -> Seen:
    """Crawls one page from `start` with `AsyncCrawl`, its first attempt failing."""
    seen = Seen()
    crawl = AsyncCrawl()
    page = crawl.label("page")
    opened = anyio.Event()
    runs: list[AsyncCrawlRun] = []

    @crawl.handle(page)
    async def write(ctx: AsyncCrawlContext[None]) -> None:
        attempt = len(seen.attempts) + 1
        staged = ctx.run.key_value_store
        # The trap: the program's own Run value, captured by its handler.
        await opened.wait()
        captured = runs[0].key_value_store
        seen.run_ids.append(ctx.run.id)
        found = [await staged.get("staged"), await staged.get("captured")]
        await staged.set("staged", attempt)
        await staged.set_bytes("bytes", bytes([attempt]), PNG)
        await captured.set("captured", attempt)
        blob = await staged.get_bytes("bytes")
        seen.attempts.append(
            {
                "found": found,
                "through_attempt": [
                    await staged.get("staged"),
                    None if blob is None else (blob.content, blob.content_type),
                ],
                "through_program": [
                    await captured.get("staged"),
                    await captured.get_bytes("bytes"),
                    await captured.get("captured"),
                ],
                "keys": [key async for key in staged.keys()],
                "deleted": [
                    await staged.delete("bytes"),
                    await staged.get_bytes("bytes"),
                ],
            }
        )
        if attempt == 1:
            raise FailedAfterItsWritesError

    foxhound = await AsyncFoxhound.connect(env=env)
    run = await foxhound.start_crawl(
        crawl, seeds=[seed(start, page)], settings=SETTINGS
    )
    runs.append(run)
    opened.set()
    await run.finish()
    seen.run_id = run.id
    seen.committed = [
        await run.key_value_store.get("staged"),
        await run.key_value_store.get_bytes("bytes"),
        await run.key_value_store.get("captured"),
    ]
    await foxhound.close()
    return seen


def _crawl_sync(env: Mapping[str, str], start: str) -> Seen:
    """Crawls one page from `start` with `Crawl`, its first attempt failing."""
    seen = Seen()
    crawl = Crawl()
    page = crawl.label("page")
    opened = threading.Event()
    runs: list[CrawlRun] = []

    @crawl.handle(page)
    def write(ctx: CrawlContext[None]) -> None:
        attempt = len(seen.attempts) + 1
        staged = ctx.run.key_value_store
        # The trap: the program's own Run value, captured by its handler.
        opened.wait()
        captured = runs[0].key_value_store
        seen.run_ids.append(ctx.run.id)
        found = [staged.get("staged"), staged.get("captured")]
        staged.set("staged", attempt)
        staged.set_bytes("bytes", bytes([attempt]), PNG)
        captured.set("captured", attempt)
        blob = staged.get_bytes("bytes")
        seen.attempts.append(
            {
                "found": found,
                "through_attempt": [
                    staged.get("staged"),
                    None if blob is None else (blob.content, blob.content_type),
                ],
                "through_program": [
                    captured.get("staged"),
                    captured.get_bytes("bytes"),
                    captured.get("captured"),
                ],
                "keys": list(staged.keys()),
                "deleted": [staged.delete("bytes"), staged.get_bytes("bytes")],
            }
        )
        if attempt == 1:
            raise FailedAfterItsWritesError

    with Foxhound.connect(env=env) as foxhound:
        run = foxhound.start_crawl(crawl, seeds=[seed(start, page)], settings=SETTINGS)
        runs.append(run)
        opened.set()
        run.finish()
        seen.run_id = run.id
        seen.committed = [
            run.key_value_store.get("staged"),
            run.key_value_store.get_bytes("bytes"),
            run.key_value_store.get("captured"),
        ]
    return seen


def _sees(attempt: int, found: list[object]) -> dict[str, object]:
    """What `attempt` sees, having first found `found`; `keys` lists committed keys."""
    return {
        "found": found,
        "through_attempt": [attempt, (bytes([attempt]), PNG)],
        "through_program": [None, None, attempt],
        "keys": ["captured"],
        "deleted": [True, None],
    }


@pytest.mark.covers(
    "story:11-1064",
    "operation:key_value_store.get",
    "operation:key_value_store.set",
    "operation:key_value_store.get_bytes",
    "operation:key_value_store.set_bytes",
    "operation:key_value_store.delete",
)
@three_ways
async def test_stages_a_write_through_ctx_run_and_commits_a_captured_runs_at_once(
    way: Way, env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """The failed attempt's staged write is gone, and its captured one stays."""
    start = await anyio.to_thread.run_sync(proving_ground.start_url, TASK)

    seen = (
        await anyio.to_thread.run_sync(_crawl_sync, env, start)
        if way is Way.SYNC
        else await _crawl_async(env, start)
    )

    assert seen.attempts == [_sees(1, [None, None]), _sees(2, [None, 1])]
    assert seen.run_ids == [seen.run_id, seen.run_id]
    assert seen.committed == [2, None, 2]
