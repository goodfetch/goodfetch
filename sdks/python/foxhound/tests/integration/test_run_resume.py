"""Crawl Run ID resume, ended, crash recovery and conflicts (issue 1493).

Tests clean stop & resume (`started_as` == `resumed`, INFO log on
`goodfetch.client.run`, no duplicate Records, no `handler_lost`), ended Run
(`started_as` == `ended`, WARNING log, no handler called), crash recovery at
`full` durability (`handler_lost` counted against `crawl.retry.max`), and typed
refusal classes (`RunInUseError`, `RunConflictError`, `RunStateUnavailableError`,
`LimitReachedError`, `LabelUnknownError`, `DatasetUnknownError`,
`DatasetKeyInvalidError`, `SettingNotApplicableError`).
"""

from __future__ import annotations

import logging
import shutil
from pathlib import Path
from typing import TYPE_CHECKING
from urllib.parse import urlsplit

import anyio
import anyio.to_thread
import pytest
from goodfetch_collie_client.generated.errors import (
    LimitReachedError,
    RunConflictError,
    RunInUseError,
    RunStateUnavailableError,
    SettingNotApplicableError,
)
from goodfetch_collie_client.generated.models import Lifecycle, StartedAs, StartInput
from goodfetch_collie_client.limits import Limits
from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncFoxhound,
    seed,
)
from goodfetch_foxhound.generated.errors import (
    DatasetKeyInvalidError,
    DatasetUnknownError,
    LabelUnknownError,
)
from goodfetch_test_support import Product as ToolProduct
from pydantic import BaseModel

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_foxhound import AsyncCrawlContext
    from goodfetch_foxhound.generated.settings import CrawlSettings
    from goodfetch_test_support import ProvingGround


class Item(BaseModel):
    """Dataset Record shape for resume tests."""

    id: str
    title: str


class ExtendedItem(BaseModel):
    """Widened Dataset Record shape accepted on resume."""

    id: str
    title: str
    note: str | None = None


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
    return 1493


def _find_run_sqlite(home: str, run_id: str) -> Path:
    root = Path(home)
    for child in root.iterdir():
        candidate = child / "runs" / run_id / "run.sqlite"
        if candidate.is_file():
            return candidate
    msg = f"run.sqlite for {run_id} not found under {home}"
    raise FileNotFoundError(msg)


def _restore_run_file(src: Path, dst: Path) -> None:
    for suffix in ("sqlite-wal", "sqlite-shm"):
        dst.with_suffix(f".{suffix}").unlink(missing_ok=True)
    shutil.copyfile(src, dst)


def _remove_run_dir(run_sqlite: Path) -> None:
    shutil.rmtree(run_sqlite.parent)


@pytest.mark.covers(
    "story:11-127",
    "story:3-176",
    "story:3-177",
    "operation:crawl.start",
    "operation:run.started_as",
    "operation:run.stop",
    "error:handler_lost",
)
@pytest.mark.anyio
async def test_resume_clean_stop_ended_logs_and_crash_handler_lost(
    env: Mapping[str, str],
    proving_ground: ProvingGround,
    tmp_path: Path,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Clean stop counts no handler_lost; ended logs warn; crash counts handler_lost."""
    first_url = await anyio.to_thread.run_sync(proving_ground.start_url, "catalog/1")
    second_url = f"{first_url}?page=2"
    base_settings: CrawlSettings = {
        **site_settings(proving_ground),
        "run_state": {"durability": "full"},
        "crawl": {"handler": {"concurrency": 1}, "retry": {"max": 1}},
    }
    zero_retry_settings: CrawlSettings = {
        **site_settings(proving_ground),
        "run_state": {"durability": "full"},
        "crawl": {"handler": {"concurrency": 1}, "retry": {"max": 0}},
    }

    # 1. Clean stop() right after page 1 pushes and enqueues page 2.
    clean_id = "py-resume-clean"
    first_ready = anyio.Event()
    release_first = anyio.Event()
    crawl1 = AsyncCrawl()
    items1 = crawl1.dataset("items", schema=Item, key=["id"])
    first1 = crawl1.label("first")
    second1 = crawl1.label("second")

    @crawl1.handle(first1)
    async def handle_first1(ctx: AsyncCrawlContext[None]) -> None:
        await ctx.push(items1, Item(id="one", title="First"))
        await ctx.enqueue(second_url, second1)
        first_ready.set()
        await release_first.wait()

    @crawl1.handle(second1)
    async def handle_second1(_ctx: AsyncCrawlContext[None]) -> None:
        await anyio.sleep_forever()

    fh1 = await AsyncFoxhound.connect(env=env)
    try:
        run1 = await fh1.start_crawl(
            crawl1,
            seeds=[seed(first_url, first1)],
            run_id=clean_id,
            settings=base_settings,
        )
        assert run1.started_as == StartedAs.new
        await first_ready.wait()
        stopped_box: list[Lifecycle] = []

        async def do_stop() -> None:
            report = await run1.stop()
            stopped_box.append(report.lifecycle)

        async with anyio.create_task_group() as tg:
            tg.start_soon(do_stop)
            await run1.key_value_store.get("__stop_barrier")
            release_first.set()
        assert stopped_box == [Lifecycle.stopped]
    finally:
        await fh1.close()

    # 2. Resume after clean stop with retry.max = 0: succeeds (no handler_lost counted).
    first_calls = 0
    second_calls = 0
    crawl2 = AsyncCrawl()
    items2 = crawl2.dataset("items", schema=Item, key=["id"])
    first2 = crawl2.label("first")
    second2 = crawl2.label("second")

    @crawl2.handle(first2)
    async def handle_first2(_ctx: AsyncCrawlContext[None]) -> None:
        nonlocal first_calls
        first_calls += 1

    @crawl2.handle(second2)
    async def handle_second2(ctx: AsyncCrawlContext[None]) -> None:
        nonlocal second_calls
        second_calls += 1
        await ctx.push(items2, Item(id="two", title="Second"))

    caplog.set_level(logging.INFO, logger="goodfetch.client.run")
    fh2 = await AsyncFoxhound.connect(env=env)
    try:
        resumed = await fh2.start_crawl(
            crawl2,
            seeds=[seed(first_url, first2)],
            run_id=clean_id,
            settings=zero_retry_settings,
        )
        assert resumed.started_as == StartedAs.resumed
        finished = await resumed.finish()
        assert finished.lifecycle == Lifecycle.finished
        assert first_calls == 0
        assert second_calls == 1
        assert finished.requests is not None
        assert not any(
            e.name == "handler_lost" for e in (finished.requests.by_error_code or [])
        )

        records = [rec.value async for rec in resumed.records(items2)]
        assert records == [
            Item(id="one", title="First"),
            Item(id="two", title="Second"),
        ]

        # 3. Start under ended Run ID -> WARNING log, handler never invoked.
        ended_calls = 0
        crawl3 = AsyncCrawl()
        crawl3.dataset("items", schema=Item, key=["id"])
        first3 = crawl3.label("first")
        second3 = crawl3.label("second")

        @crawl3.handle(first3)
        @crawl3.handle(second3)
        async def handle_ended(_ctx: AsyncCrawlContext[None]) -> None:
            nonlocal ended_calls
            ended_calls += 1

        ended = await fh2.start_crawl(
            crawl3,
            seeds=[seed(first_url, first3)],
            run_id=clean_id,
            settings=base_settings,
        )
        assert ended.started_as == StartedAs.ended
        report = await ended.report()
        assert report.lifecycle == Lifecycle.finished
        assert ended_calls == 0
    finally:
        await fh2.close()

    run_records = [r for r in caplog.records if r.name == "goodfetch.client.run"]
    assert [(r.levelno, getattr(r, "started_as", None)) for r in run_records] == [
        (logging.INFO, "resumed"),
        (logging.WARNING, "ended"),
    ]

    # 4. Crash recovery at full durability: in-flight attempt counts handler_lost.
    crash_id = "py-resume-crash"
    checkpoint = tmp_path / "in-flight.sqlite"
    crash_in_flight = anyio.Event()
    crash_crawl1 = AsyncCrawl()
    crash_items1 = crash_crawl1.dataset("items", schema=Item, key=["id"])
    cf1 = crash_crawl1.label("first")
    cs1 = crash_crawl1.label("second")

    @crash_crawl1.handle(cf1)
    async def handle_cf1(ctx: AsyncCrawlContext[None]) -> None:
        await ctx.push(crash_items1, Item(id="one", title="First"))
        await ctx.enqueue(second_url, cs1)

    @crash_crawl1.handle(cs1)
    async def handle_cs1(_ctx: AsyncCrawlContext[None]) -> None:
        crash_in_flight.set()
        await anyio.sleep_forever()

    fh_c1 = await AsyncFoxhound.connect(env=env)
    try:
        c_run = await fh_c1.start_crawl(
            crash_crawl1,
            seeds=[seed(first_url, cf1)],
            run_id=crash_id,
            settings=base_settings,
        )
        await crash_in_flight.wait()
        await fh_c1.call(
            "run.checkpoint", {"run_id": crash_id, "location": str(checkpoint)}
        )
        await c_run.close()
    finally:
        await fh_c1.close()

    run_sqlite = _find_run_sqlite(env["GOODFETCH_HOME"], crash_id)
    _restore_run_file(checkpoint, run_sqlite)

    crash_first_calls = 0
    crash_second_calls = 0
    crash_crawl2 = AsyncCrawl()
    crash_items2 = crash_crawl2.dataset("items", schema=Item, key=["id"])
    cf2 = crash_crawl2.label("first")
    cs2 = crash_crawl2.label("second")

    @crash_crawl2.handle(cf2)
    async def handle_cf2(_ctx: AsyncCrawlContext[None]) -> None:
        nonlocal crash_first_calls
        crash_first_calls += 1

    @crash_crawl2.handle(cs2)
    async def handle_cs2(ctx: AsyncCrawlContext[None]) -> None:
        nonlocal crash_second_calls
        crash_second_calls += 1
        await ctx.push(crash_items2, Item(id="two", title="Second"))

    fh_c2 = await AsyncFoxhound.connect(env=env)
    try:
        resumed_c = await fh_c2.start_crawl(
            crash_crawl2,
            seeds=[seed(first_url, cf2)],
            run_id=crash_id,
            settings=base_settings,
        )
        assert resumed_c.started_as == StartedAs.resumed
        finished_c = await resumed_c.finish()
        assert finished_c.lifecycle == Lifecycle.finished
        assert crash_first_calls == 0
        assert crash_second_calls == 1
        recs = [rec.value async for rec in resumed_c.records(crash_items2)]
        assert len(recs) == 2
    finally:
        await fh_c2.close()

    # With crawl.retry.max = 0, crashed state fails immediately with handler_lost.
    _restore_run_file(checkpoint, run_sqlite)
    fh_zero = await AsyncFoxhound.connect(env=env)
    try:
        resumed_zero = await fh_zero.start_crawl(
            crash_crawl2,
            seeds=[seed(first_url, cf2)],
            run_id=crash_id,
            settings=zero_retry_settings,
        )
        assert resumed_zero.started_as == StartedAs.resumed
        report_zero = await resumed_zero.finish()
        assert report_zero.requests is not None
        assert any(
            e.name == "handler_lost" and e.count == 1
            for e in (report_zero.requests.by_error_code or [])
        )
    finally:
        await fh_zero.close()


@pytest.mark.covers(
    "story:3-167",
    "story:3-172",
    "story:6-272",
    "operation:crawl.start",
    "operation:run.start",
    "error:run_in_use",
    "error:run_conflict",
    "error:run_state_unavailable",
    "error:limit_reached",
    "error:label_unknown",
    "error:dataset_unknown",
    "error:dataset_key_invalid",
    "error:setting_not_applicable",
)
@pytest.mark.anyio
async def test_start_refusals_and_resume_validation(
    env: Mapping[str, str],
    proving_ground: ProvingGround,
) -> None:
    """Refuses in-use, conflicts, corrupt state, limits and invalid resumes."""
    first_url = await anyio.to_thread.run_sync(proving_ground.start_url, "catalog/1")
    second_url = f"{first_url}?page=2"
    p1 = urlsplit(first_url)
    first_origin = f"{p1.scheme}://{p1.netloc}"
    scoped_settings: CrawlSettings = {
        **site_settings(proving_ground),
        "run_state": {"durability": "full"},
        "crawl": {
            "handler": {"concurrency": 1},
            "scope": {"include": [f"{first_origin}/**"]},
        },
    }

    first_ready = anyio.Event()
    release_first = anyio.Event()
    crawl = AsyncCrawl()
    items = crawl.dataset("items", schema=Item, key=["id"])
    first = crawl.label("first")
    second = crawl.label("second")

    @crawl.handle(first)
    async def handle_first(ctx: AsyncCrawlContext[None]) -> None:
        await ctx.push(items, Item(id="one", title="First"))
        await ctx.enqueue(second_url, second)
        first_ready.set()
        await release_first.wait()

    @crawl.handle(second)
    async def handle_second(_ctx: AsyncCrawlContext[None]) -> None:
        return None

    run_id = "py-validation-run"
    fh = await AsyncFoxhound.connect(env=env)
    try:
        live = await fh.start_crawl(
            crawl,
            seeds=[seed(first_url, first)],
            run_id=run_id,
            settings=scoped_settings,
        )
        await first_ready.wait()

        with pytest.raises(RunInUseError) as in_use:
            await fh.start_crawl(
                crawl,
                seeds=[seed(first_url, first)],
                run_id=run_id,
                settings=scoped_settings,
            )
        assert in_use.value.retryable is True

        async def stop_live() -> None:
            await live.stop()

        async with anyio.create_task_group() as tg:
            tg.start_soon(stop_live)
            await live.key_value_store.get("__stop_barrier")
            release_first.set()

        with pytest.raises(RunConflictError) as seed_conflict:
            await fh.start_crawl(
                crawl,
                seeds=[seed(second_url, first)],
                run_id=run_id,
                settings=scoped_settings,
            )
        assert seed_conflict.value.typed_details is not None
        assert StartInput.seeds in seed_conflict.value.typed_details.differences

        plan_id = "py-plan-conflict"
        plan_run = await fh.start(run_id=plan_id, plan="a.plan.json")
        await plan_run.stop()
        with pytest.raises(RunConflictError) as plan_conflict:
            await fh.start(run_id=plan_id, plan="b.plan.json")
        assert plan_conflict.value.typed_details is not None
        assert StartInput.plan in plan_conflict.value.typed_details.differences

        missing_label = AsyncCrawl()
        missing_label.dataset("items", schema=Item, key=["id"])
        ml_first = missing_label.label("first")

        @missing_label.handle(ml_first)
        async def _ml(_ctx: AsyncCrawlContext[None]) -> None:
            return None

        with pytest.raises(LabelUnknownError):
            await fh.start_crawl(
                missing_label,
                seeds=[seed(first_url, ml_first)],
                run_id=run_id,
                settings=scoped_settings,
            )

        missing_ds = AsyncCrawl()
        md_first = missing_ds.label("first")
        md_second = missing_ds.label("second")

        @missing_ds.handle(md_first)
        @missing_ds.handle(md_second)
        async def _md(_ctx: AsyncCrawlContext[None]) -> None:
            return None

        with pytest.raises(DatasetUnknownError):
            await fh.start_crawl(
                missing_ds,
                seeds=[seed(first_url, md_first)],
                run_id=run_id,
                settings=scoped_settings,
            )

        wrong_key = AsyncCrawl()
        wrong_key.dataset("items", schema=Item, key=["title"])
        wk_first = wrong_key.label("first")
        wk_second = wrong_key.label("second")

        @wrong_key.handle(wk_first)
        @wrong_key.handle(wk_second)
        async def _wk(_ctx: AsyncCrawlContext[None]) -> None:
            return None

        with pytest.raises(DatasetKeyInvalidError):
            await fh.start_crawl(
                wrong_key,
                seeds=[seed(first_url, wk_first)],
                run_id=run_id,
                settings=scoped_settings,
            )

        with pytest.raises(SettingNotApplicableError):
            await fh.start_crawl(
                crawl,
                seeds=[seed(first_url, first)],
                run_id=run_id,
                settings={
                    **scoped_settings,
                    "crawl": {
                        "handler": {"concurrency": 1},
                        "scope": {"include": ["https://other.test/**"]},
                    },
                },
            )

        widened = AsyncCrawl()
        wide_items = widened.dataset("items", schema=ExtendedItem, key=["id"])
        w_first = widened.label("first")
        w_second = widened.label("second")
        w_third = widened.label("third")

        @widened.handle(w_first)
        @widened.handle(w_third)
        async def _w_noop(_ctx: AsyncCrawlContext[None]) -> None:
            return None

        @widened.handle(w_second)
        async def _w_second(ctx: AsyncCrawlContext[None]) -> None:
            await ctx.push(
                wide_items, ExtendedItem(id="two", title="Second", note="widened")
            )

        resumed = await fh.start_crawl(
            widened,
            seeds=[seed(first_url, w_first)],
            run_id=run_id,
            settings=scoped_settings,
        )
        assert resumed.started_as == StartedAs.resumed
        finished = await resumed.finish()
        assert finished.lifecycle == Lifecycle.finished
    finally:
        await fh.close()

    _remove_run_dir(_find_run_sqlite(env["GOODFETCH_HOME"], "py-plan-conflict"))
    fh_corrupt = await AsyncFoxhound.connect(env=env)
    try:
        with pytest.raises(RunStateUnavailableError) as unavailable:
            await fh_corrupt.start(run_id="py-plan-conflict", plan="a.plan.json")
        assert unavailable.value.retryable is False
    finally:
        await fh_corrupt.close()

    fh_capped = await AsyncFoxhound.connect(env=env, limits=Limits(requests=0))
    try:
        with pytest.raises(LimitReachedError) as limited:
            await fh_capped.start_crawl(
                crawl,
                seeds=[seed(first_url, first)],
                settings=site_settings(proving_ground),
            )
        assert limited.value.typed_details is not None
        assert limited.value.typed_details.usage_kind == "requests"
    finally:
        await fh_capped.close()
