"""Crawl request headers on seeds and `enqueue` (#3660)."""

from __future__ import annotations

from datetime import timedelta
from typing import TYPE_CHECKING, cast

import anyio
import pytest
from goodfetch_collie_client.generated.errors import HeaderForbiddenError
from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncFoxhound,
    Crawl,
    Foxhound,
    seed,
)
from goodfetch_test_support import Product as ToolProduct

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_foxhound import AsyncCrawlContext, CrawlContext
    from goodfetch_foxhound.generated.models import Enqueued
    from goodfetch_foxhound.generated.settings import FoxhoundSettings
    from goodfetch_test_support import ProvingGround

HOST = "access.test"
FORBIDDEN: tuple[tuple[str, str], ...] = (
    ("Host", "other.test"),
    ("Cookie", "session=disallowed"),
    ("Authorization", "disallowed"),
    ("Proxy-Authorization", "disallowed"),
    ("X-HTTP-Method-Override", "DELETE"),
)


@pytest.fixture
def product() -> ToolProduct:
    """The test runs Foxhound's executable."""
    return ToolProduct.FOXHOUND


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground seed."""
    return 3660


def _fast_site_settings(proving_ground: ProvingGround) -> FoxhoundSettings:
    resolve: dict[str, list[str]] = {
        host: [address] for host, address in proving_ground.resolve.items()
    }
    return {
        "fetch": {"tier": "http"},
        "network": {"resolve": resolve},
        "egress": {"private_ranges": list(resolve)},
        "politeness": {"domain": {"concurrency": 8, "delay": timedelta(0)}},
    }


def _observed_header(
    observations: Mapping[str, object], target: str, name: str
) -> str | None:
    expected = name.lower()
    requests = cast("list[dict[str, object]]", observations["requests"])
    for req in requests:
        if req.get("host") == HOST and req.get("target") == target:
            headers = cast("list[list[str]]", req.get("headers", []))
            for key, value in headers:
                if key.lower() == expected:
                    return value
    return None


@pytest.mark.covers(
    "story:11-1019",
    "operation:crawl_request.enqueue",
    "error:header_forbidden",
)
@pytest.mark.anyio
async def test_seed_and_enqueue_headers_and_forbidden_rejection_async(
    env: Mapping[str, str],
    proving_ground: ProvingGround,
) -> None:
    """Headers reach Proving ground, expose on request, and reject forbidden."""
    root_url = proving_ground.url(HOST, "/corpus/article/1")
    child_url = proving_ground.url(HOST, "/corpus/article/2")
    blocked_url = proving_ground.url(HOST, "/corpus/article/3")

    foxhound = await AsyncFoxhound.connect(
        env=env, settings=_fast_site_settings(proving_ground)
    )
    try:
        seen_headers: dict[str, Mapping[str, str]] = {}
        duplicate_answer: list[Enqueued] = []
        enqueue_errors: list[tuple[str, str]] = []

        crawl = AsyncCrawl()
        root = crawl.label("root")
        child = crawl.label("child")

        @crawl.handle(root)
        async def handle_root(ctx: AsyncCrawlContext[None]) -> None:
            seen_headers["root"] = dict(ctx.request.headers)
            first = await ctx.enqueue(
                child_url, child, headers={"X-Py-Enqueue": "py-enqueued"}
            )
            assert first.root.kind == "added"
            dup = await ctx.enqueue(
                child_url, child, headers={"X-Py-Enqueue": "other-val"}
            )
            duplicate_answer.append(dup)
            for name, value in FORBIDDEN:
                with pytest.raises(HeaderForbiddenError) as exc_info:
                    await ctx.enqueue(blocked_url, child, headers={name: value})
                enqueue_errors.append((name, exc_info.value.code))

        @crawl.handle(child)
        async def handle_child(ctx: AsyncCrawlContext[None]) -> None:
            seen_headers["child"] = dict(ctx.request.headers)

        for name, value in FORBIDDEN:
            with pytest.raises(HeaderForbiddenError):
                await foxhound.start_crawl(
                    crawl, seeds=[seed(blocked_url, child, headers={name: value})]
                )

        run = await foxhound.start_crawl(
            crawl, seeds=[seed(root_url, root, headers={"X-Py-Seed": "py-seed"})]
        )
        report = await run.finish()
        assert report.lifecycle == "finished"

        observations = await anyio.to_thread.run_sync(proving_ground.observations)
        assert (
            _observed_header(observations, "/corpus/article/1", "x-py-seed")
            == "py-seed"
        )
        assert (
            _observed_header(observations, "/corpus/article/2", "x-py-enqueue")
            == "py-enqueued"
        )
        requests = cast("list[dict[str, object]]", observations["requests"])
        assert all(req.get("target") != "/corpus/article/3" for req in requests)
        assert duplicate_answer[0].root.kind == "not_added"
        assert duplicate_answer[0].root.reason == "duplicate"
        assert seen_headers["root"] == {"X-Py-Seed": "py-seed"}
        assert seen_headers["child"] == {"X-Py-Enqueue": "py-enqueued"}
        assert enqueue_errors == [(name, "header_forbidden") for name, _ in FORBIDDEN]
    finally:
        await foxhound.close()


@pytest.mark.covers(
    "story:11-1019",
    "operation:crawl_request.enqueue",
    "error:header_forbidden",
)
def test_seed_and_enqueue_headers_sync(
    env: Mapping[str, str],
    proving_ground: ProvingGround,
) -> None:
    """Sync Foxhound carries seed and enqueue headers to Proving ground."""
    root_url = proving_ground.url(HOST, "/corpus/article/1")
    child_url = proving_ground.url(HOST, "/corpus/article/2")

    foxhound = Foxhound.connect(env=env, settings=_fast_site_settings(proving_ground))
    try:
        crawl = Crawl()
        root = crawl.label("root")
        child = crawl.label("child")

        @crawl.handle(root)
        def handle_root(ctx: CrawlContext[None]) -> None:
            ans = ctx.enqueue(
                child_url, child, headers={"X-Py-Sync-Enqueue": "sync-enq"}
            )
            assert ans.root.kind == "added"

        @crawl.handle(child)
        def handle_child(ctx: CrawlContext[None]) -> None:
            assert ctx.request.headers == {"X-Py-Sync-Enqueue": "sync-enq"}

        run = foxhound.start_crawl(
            crawl, seeds=[seed(root_url, root, headers={"X-Py-Sync-Seed": "sync-seed"})]
        )
        report = run.finish()
        assert report.lifecycle == "finished"

        observations = proving_ground.observations()
        assert (
            _observed_header(observations, "/corpus/article/1", "x-py-sync-seed")
            == "sync-seed"
        )
        assert (
            _observed_header(observations, "/corpus/article/2", "x-py-sync-enqueue")
            == "sync-enq"
        )
    finally:
        foxhound.close()
