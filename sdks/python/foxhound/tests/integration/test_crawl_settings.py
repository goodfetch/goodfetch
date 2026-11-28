"""Crawl-space settings and `explain_settings()` for Foxhound's keys (#1500)."""

from __future__ import annotations

from datetime import timedelta
from pathlib import Path
from typing import TYPE_CHECKING

import pytest
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


@pytest.fixture
def product() -> ToolProduct:
    """The test runs Foxhound's executable."""
    return ToolProduct.FOXHOUND


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground seed."""
    return 1500


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


def _write_policy(env: Mapping[str, str]) -> None:
    home = env.get("GOODFETCH_HOME")
    assert home is not None
    per_major = Path(home) / "0.0"
    per_major.mkdir(parents=True, exist_ok=True)
    (per_major / "operator-policy.toml").write_text(
        'version = "0.1"\n\n'
        "[lock.crawl]\nmax_depth = 2\n\n"
        "[cap.crawl]\nmax_requests = 10\n",
        encoding="utf-8",
    )


@pytest.mark.covers(
    "story:6-227",
    "story:6-228",
    "story:6-314",
    "setting:crawl.max_depth",
    "setting:crawl.max_requests",
    "setting:crawl.max_url_length",
    "setting:crawl.max_path_segments",
    "setting:crawl.max_segment_repeats",
    "operation:config.explain",
)
@pytest.mark.anyio
async def test_url_caps_none_clears_and_explain_settings_matches_report_async(
    env: Mapping[str, str],
    proving_ground: ProvingGround,
) -> None:
    """Caps refuse with not-added reasons, `None` clears, `explain_settings` matches."""
    _write_policy(env)
    settings = _fast_site_settings(proving_ground)
    foxhound = await AsyncFoxhound.connect(env=env, settings=settings)

    seed_url = proving_ground.url("traps.test", "/archive/")
    long_url = proving_ground.url(
        "traps.test",
        "/archive/very-long-segment-that-exceeds-the-configured-length-cap",
    )
    deep_url = proving_ground.url("traps.test", "/a/b/c/d")
    repeat_url = proving_ground.url("traps.test", "/x/x/y")

    answers: list[Enqueued] = []
    crawl = AsyncCrawl()
    root = crawl.label("root")
    child = crawl.label("child")

    @crawl.handle(root)
    async def _handle_root(ctx: AsyncCrawlContext[None]) -> None:
        answers.append(await ctx.enqueue(long_url, child))
        answers.append(await ctx.enqueue(deep_url, child))
        answers.append(await ctx.enqueue(repeat_url, child))

    @crawl.handle(child)
    async def _handle_child(ctx: AsyncCrawlContext[None]) -> None:
        del ctx

    capped = await foxhound.start_crawl(
        crawl,
        seeds=[seed(seed_url, root)],
        settings={
            "crawl": {
                "max_url_length": len(seed_url) + 10,
                "max_path_segments": 3,
                "max_segment_repeats": 1,
            }
        },
    )
    explained = await capped.explain_settings()
    by_key = {entry.key: entry for entry in explained.settings or []}
    assert by_key["crawl.max_depth"].value == 2
    assert by_key["crawl.max_depth"].layer == "operator_policy"
    assert by_key["crawl.max_depth"].locked_by == "operator_policy"
    assert by_key["crawl.max_requests"].value == 10
    assert by_key["crawl.max_requests"].layer == "operator_policy"
    assert by_key["crawl.max_requests"].cap == 10
    assert by_key["crawl.max_url_length"].value == len(seed_url) + 10
    assert by_key["crawl.max_url_length"].layer == "run"

    capped_report = await capped.finish()
    assert capped_report.completeness == "unverified"
    assert capped_report.settings == explained.settings
    assert [
        ans.root.reason if ans.root.kind == "not_added" else ans.root.kind
        for ans in answers
    ] == ["max_url_length", "max_path_segments", "max_segment_repeats"]

    answers.clear()
    uncapped = await foxhound.start_crawl(
        crawl,
        seeds=[seed(seed_url, root)],
        settings={
            "crawl": {
                "max_url_length": None,
                "max_path_segments": None,
                "max_segment_repeats": None,
            }
        },
    )
    uncapped_report = await uncapped.finish()
    assert uncapped_report.completeness == "complete"
    assert [ans.root.kind for ans in answers] == ["added", "added", "added"]

    await foxhound.close()


@pytest.mark.covers(
    "story:6-227",
    "story:6-228",
    "story:6-314",
    "setting:crawl.max_depth",
    "setting:crawl.max_requests",
    "setting:crawl.max_url_length",
    "operation:config.explain",
)
def test_url_caps_and_explain_settings_sync(
    env: Mapping[str, str],
    proving_ground: ProvingGround,
) -> None:
    """Sync `explain_settings()` reports policy locks and equals `report.settings`."""
    _write_policy(env)
    foxhound = Foxhound.connect(env=env, settings=_fast_site_settings(proving_ground))
    seed_url = proving_ground.url("traps.test", "/archive/")

    crawl = Crawl()
    page = crawl.label("page")

    @crawl.handle(page)
    def _handle(ctx: CrawlContext[None]) -> None:
        del ctx

    run = foxhound.start_crawl(
        crawl,
        seeds=[seed(seed_url, page)],
        settings={"crawl": {"max_url_length": 4_096}},
    )
    explained = run.explain_settings()
    by_key = {entry.key: entry for entry in explained.settings or []}
    assert by_key["crawl.max_depth"].value == 2
    assert by_key["crawl.max_depth"].locked_by == "operator_policy"
    assert by_key["crawl.max_url_length"].value == 4_096
    assert by_key["crawl.max_url_length"].layer == "run"

    report = run.finish()
    assert report.settings == explained.settings
    foxhound.close()
