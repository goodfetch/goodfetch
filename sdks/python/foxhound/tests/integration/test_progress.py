"""`progress()` says why a slow crawl is slow (issue 1496; ADRs 0040, 0086, 0102)."""

from __future__ import annotations

from datetime import timedelta
from typing import TYPE_CHECKING

import anyio
import pytest
from goodfetch_foxhound import AsyncCrawl, AsyncFoxhound, Crawl, Foxhound, seed, sitemap
from goodfetch_test_support import Product as ToolProduct

if TYPE_CHECKING:
    from goodfetch_foxhound import AsyncCrawlContext, CrawlContext
    from goodfetch_foxhound.generated.models import CrawlRunProgress
    from goodfetch_foxhound.generated.settings import FoxhoundSettings
    from goodfetch_test_support import ProvingGround

ACCESS_HOST = "access.test"
EDGE_HOST = "fetch-edge.test"
SITEMAP_HOST = "sitemap-seed.test"


@pytest.fixture
def product() -> ToolProduct:
    """The test runs Foxhound's executable."""
    return ToolProduct.FOXHOUND


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground seed."""
    return 1496


def _resolve_settings(proving_ground: ProvingGround) -> FoxhoundSettings:
    resolve: dict[str, list[str]] = {
        host: [address] for host, address in proving_ground.resolve.items()
    }
    return {
        "fetch": {"tier": "http"},
        "network": {"resolve": resolve},
        "egress": {"private_ranges": list(resolve)},
    }


@pytest.mark.covers(
    "story:3-162",
    "story:6-207",
    "story:11-129",
    "story:11-1080",
    "operation:run.progress",
)
@pytest.mark.anyio
async def test_progress_waits_concurrency_sitemaps_idle_and_coalescing_async(
    env: dict[str, str], proving_ground: ProvingGround
) -> None:
    """Covers wait groups, concurrency, sitemaps, idle, immediate first, coalescing."""
    async with await AsyncFoxhound.connect(
        env=env, settings=_resolve_settings(proving_ground)
    ) as foxhound:
        # 1. Default politeness waits + concurrency layer & governor allowance.
        crawl = AsyncCrawl()
        page = crawl.label("page")

        @crawl.handle(page)
        async def _slow(_ctx: AsyncCrawlContext[None]) -> None:
            await anyio.sleep(0.12)

        async with await foxhound.start_crawl(
            crawl,
            seeds=[
                seed(proving_ground.url(ACCESS_HOST, "/corpus/article/1"), page),
                seed(proving_ground.url(ACCESS_HOST, "/corpus/article/2"), page),
                seed(proving_ground.url(ACCESS_HOST, "/corpus/article/3"), page),
            ],
        ) as run:
            saw_wait = False
            saw_next_admitted = False
            saw_concurrency = False
            async for step in run.progress():
                for group in step.waiting or []:
                    if group.reason in ("domain_concurrency", "domain_delay"):
                        assert any(d.domain == ACCESS_HOST for d in group.domains)
                        assert group.setting is not None
                        assert group.setting.layer == "default"
                        saw_wait = True
                        if any(d.next_admitted_at is not None for d in group.domains):
                            saw_next_admitted = True
                conc = step.concurrency
                if (
                    conc is not None
                    and conc.handler_concurrency == 16
                    and conc.handler_concurrency_layer == "default"
                    and conc.governor is not None
                    and conc.allowance_set_by is not None
                ):
                    saw_concurrency = True
            assert saw_wait is True
            assert saw_next_admitted is True
            assert saw_concurrency is True

        # 2. 503 with Retry-After -> `backoff`.
        backoff_crawl = AsyncCrawl()
        backoff_page = backoff_crawl.label("page")

        @backoff_crawl.handle(backoff_page)
        async def _noop_backoff(_ctx: AsyncCrawlContext[None]) -> None:
            return None

        async with await foxhound.start_crawl(
            backoff_crawl,
            seeds=[
                seed(
                    proving_ground.url(EDGE_HOST, "/retry-after/503/seconds"),
                    backoff_page,
                ),
                seed(proving_ground.url(EDGE_HOST, "/after-503"), backoff_page),
            ],
            settings={
                "politeness": {"domain": {"concurrency": 1, "delay": timedelta(0)}}
            },
        ) as backoff_run:
            saw_backoff = False
            async for step in backoff_run.progress():
                for group in step.waiting or []:
                    if group.reason == "backoff" and any(
                        d.domain == EDGE_HOST and d.next_admitted_at is not None
                        for d in group.domains
                    ):
                        saw_backoff = True
            assert saw_backoff is True

        # 3. Disallowing robots.txt -> `robots_txt` skip reason.
        robots_crawl = AsyncCrawl()
        robots_page = robots_crawl.label("page")

        @robots_crawl.handle(robots_page)
        async def _noop_robots(_ctx: AsyncCrawlContext[None]) -> None:
            return None

        async with await foxhound.start_crawl(
            robots_crawl,
            seeds=[seed(proving_ground.url(EDGE_HOST, "/private/secret"), robots_page)],
        ) as robots_run:
            last_robots: CrawlRunProgress | None = None
            async for step in robots_run.progress():
                last_robots = step
            assert last_robots is not None
            assert last_robots.requests is not None
            assert last_robots.requests.skipped == 1
            assert any(
                entry.name == "robots_txt" and entry.count == 1
                for entry in (last_robots.requests.by_skip_reason or [])
            )

    # 4. Sitemaps, keep-alive idle, immediate first value, and coalescing under load.
    fast_settings = _resolve_settings(proving_ground)
    fast_settings["politeness"] = {"domain": {"concurrency": 16, "delay": timedelta(0)}}
    async with await AsyncFoxhound.connect(env=env, settings=fast_settings) as fast:
        sm_crawl = AsyncCrawl()
        book = sm_crawl.label("book")
        shelf = sm_crawl.label("shelf")
        author = sm_crawl.label("author")
        for label in (book, shelf, author):

            @sm_crawl.handle(label)
            async def _noop_sm(_ctx: AsyncCrawlContext[None]) -> None:
                return None

        async with await fast.start_crawl(
            sm_crawl,
            seeds=[
                seed(proving_ground.url(SITEMAP_HOST, "/books/1"), book),
                sitemap(
                    proving_ground.url(SITEMAP_HOST, "/sitemap-index.xml"),
                    [
                        ("**/books/*", book),
                        ("**/books/**", shelf),
                        ("**/authors/*", author),
                    ],
                ),
            ],
        ) as sm_run:
            last_sm: CrawlRunProgress | None = None
            async for step in sm_run.progress():
                last_sm = step
            assert last_sm is not None
            assert last_sm.sitemaps is not None
            sm_report = last_sm.sitemaps[0]
            assert sm_report.added > 0
            assert sm_report.not_added > 0
            reasons = {entry.name for entry in (sm_report.not_added_by_reason or [])}
            assert {"duplicate", "no_rule", "out_of_scope"}.issubset(reasons)

        keep_crawl = AsyncCrawl()
        keep_page = keep_crawl.label("page")

        @keep_crawl.handle(keep_page)
        async def _noop_keep(_ctx: AsyncCrawlContext[None]) -> None:
            return None

        async with await fast.start_crawl(
            keep_crawl,
            seeds=[seed("/relative-only", keep_page)],
            settings={"crawl": {"keep_alive": True}},
        ) as keep_run:
            async with keep_run.progress() as progress:
                first = await progress.__anext__()
                assert first.lifecycle == "running"
                assert first.idle is True
            await keep_run.finish()

        burst_crawl = AsyncCrawl()
        burst_page = burst_crawl.label("page")

        @burst_crawl.handle(burst_page)
        async def _noop_burst(_ctx: AsyncCrawlContext[None]) -> None:
            return None

        async with await fast.start_crawl(
            burst_crawl,
            seeds=[
                seed(
                    proving_ground.url(ACCESS_HOST, f"/corpus/article/1?burst={i}"),
                    burst_page,
                )
                for i in range(1, 31)
            ],
        ) as burst_run:
            async with burst_run.progress() as progress:
                first = await progress.__anext__()
                report = await burst_run.finish()
                assert report.lifecycle == "finished"
                delivered = 1
                final_succeeded = (
                    0 if first.requests is None else first.requests.succeeded
                )
                if not first.last:
                    async for step in progress:
                        delivered += 1
                        final_succeeded = (
                            0 if step.requests is None else step.requests.succeeded
                        )
            assert final_succeeded == 30
            assert delivered < 30


@pytest.mark.covers(
    "story:3-162", "story:11-129", "story:11-1080", "operation:run.progress"
)
def test_progress_sync_twin(env: dict[str, str], proving_ground: ProvingGround) -> None:
    """Sync `Foxhound` reads `CrawlRunProgress` with `idle: True`."""
    crawl = Crawl()
    page = crawl.label("page")

    @crawl.handle(page)
    def _noop(_ctx: CrawlContext[None]) -> None:
        return None

    with Foxhound.connect(
        env=env, settings=_resolve_settings(proving_ground)
    ) as foxhound:
        with foxhound.start_crawl(
            crawl,
            seeds=[seed("/relative-only", page)],
            settings={"crawl": {"keep_alive": True}},
        ) as run:
            with run.progress() as progress:
                first = next(progress)
                assert first.lifecycle == "running"
                assert first.idle is True
            report = run.finish()
            assert report.lifecycle == "finished"
