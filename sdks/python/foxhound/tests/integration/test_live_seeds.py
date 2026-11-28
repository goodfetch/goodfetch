"""Seeds of a running crawl: `seeds()`, `add_seed`, `add_seeds` and keep-alive."""

from __future__ import annotations

from datetime import timedelta
from typing import TYPE_CHECKING

import anyio
import pytest
from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncFoxhound,
    Crawl,
    Foxhound,
    seed,
)
from goodfetch_foxhound.generated.errors import PayloadInvalidError
from goodfetch_foxhound.generated.models import Enqueued, Enqueued1, RequestSeed
from goodfetch_test_support import Product as ToolProduct
from pydantic import BaseModel, Field

if TYPE_CHECKING:
    from collections.abc import AsyncIterator

    from goodfetch_collie_client.json_values import JSONObject
    from goodfetch_foxhound import AsyncCrawlContext, CrawlContext
    from goodfetch_foxhound.generated.settings import FoxhoundSettings
    from goodfetch_test_support import ProvingGround

HOST = "access.test"


class Shelf(BaseModel):
    """Typed Payload model for shelf pages."""

    number: int = Field(ge=1)


@pytest.fixture
def product() -> ToolProduct:
    """The test runs Foxhound's executable."""
    return ToolProduct.FOXHOUND


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground seed."""
    return 1494


def _fast_site_settings(proving_ground: ProvingGround) -> FoxhoundSettings:
    resolve: dict[str, list[str]] = {
        host: [address] for host, address in proving_ground.resolve.items()
    }
    return {
        "fetch": {"tier": "http"},
        "network": {"resolve": resolve},
        "egress": {"private_ranges": list(resolve)},
        "politeness": {"domain": {"concurrency": 16, "delay": timedelta(0)}},
    }


@pytest.mark.covers(
    "story:6-208",
    "story:6-209",
    "story:6-210",
    "story:11-1017",
    "story:11-1018",
    "operation:run.seeds",
    "operation:run.add_seed",
    "operation:run.add_seeds",
    "setting:crawl.keep_alive",
    "error:payload_invalid",
)
@pytest.mark.anyio
async def test_seeds_keep_alive_staging_and_streaming_async(
    env: dict[str, str], proving_ground: ProvingGround
) -> None:
    """Covers seeds, resume, keep-alive, timeout, staging, add_seeds, payloads."""
    root_url = proving_ground.url(HOST, "/corpus/article/1")
    second_url = proving_ground.url(HOST, "/corpus/article/2")
    discarded_single = proving_ground.url(HOST, "/corpus/article/10")
    discarded_stream = proving_ground.url(HOST, "/corpus/article/11")
    committed_single = proving_ground.url(HOST, "/corpus/article/12")
    committed_stream = proving_ground.url(HOST, "/corpus/article/13")

    async with await AsyncFoxhound.connect(
        env=env, settings=_fast_site_settings(proving_ground)
    ) as foxhound:
        # 1. Valid/duplicate/invalid seeds + resume preserves `seeds()` without RPC.
        run_id = "py-live-seeds-resume-1494"
        entered = anyio.Event()
        release = anyio.Event()
        crawl = AsyncCrawl()
        page = crawl.label("page")

        @crawl.handle(page)
        async def _on_page(_ctx: AsyncCrawlContext[None]) -> None:
            entered.set()
            await release.wait()

        async with await foxhound.start_crawl(
            crawl,
            run_id=run_id,
            seeds=[
                seed(root_url, page),
                seed(root_url, page),
                seed("/relative-not-a-url", page),
            ],
        ) as run:
            assert run.started_as == "new"
            initial_seeds = run.seeds()
            assert len(initial_seeds) == 3
            assert isinstance(initial_seeds[0].root, Enqueued1)
            assert initial_seeds[0].root.kind == "added"
            assert initial_seeds[1].root.kind == "not_added"
            assert initial_seeds[2].root.kind == "not_added"
            await entered.wait()
            stopped_box: list[str] = []

            async def _do_stop() -> None:
                report = await run.stop()
                stopped_box.append(report.lifecycle)

            async with anyio.create_task_group() as tg:
                tg.start_soon(_do_stop)
                await run.key_value_store.get("__stop_barrier")
                release.set()
            assert stopped_box == ["stopped"]

        resumed_crawl = AsyncCrawl()
        resumed_page = resumed_crawl.label("page")

        @resumed_crawl.handle(resumed_page)
        async def _on_resumed(_ctx: AsyncCrawlContext[None]) -> None:
            return None

        async with await foxhound.start_crawl(
            resumed_crawl,
            run_id=run_id,
            seeds=[
                seed(root_url, resumed_page),
                seed(root_url, resumed_page),
                seed("/relative-not-a-url", resumed_page),
            ],
        ) as resumed:
            assert resumed.started_as == "resumed"
            assert resumed.seeds() == initial_seeds
            resumed_report = await resumed.finish()
            assert resumed_report.lifecycle == "finished"

        # 2. All seeds refused ends at once unless `crawl.keep_alive`.
        empty_crawl = AsyncCrawl()
        empty_page = empty_crawl.label("page")

        @empty_crawl.handle(empty_page)
        async def _on_empty(_ctx: AsyncCrawlContext[None]) -> None:
            return None

        async with await foxhound.start_crawl(
            empty_crawl,
            seeds=[seed("/relative-1", empty_page), seed("/relative-2", empty_page)],
        ) as empty_run:
            assert len(empty_run.seeds()) == 2
            assert all(item.root.kind == "not_added" for item in empty_run.seeds())
            empty_report = await empty_run.finish()
            assert empty_report.lifecycle == "finished"

        # 3. Keep-alive with all seeds refused stays idle and accepts `add_seed`.
        seen: list[str] = []
        keep_crawl = AsyncCrawl()
        keep_page = keep_crawl.label("page")

        @keep_crawl.handle(keep_page)
        async def _on_keep(ctx: AsyncCrawlContext[None]) -> None:
            seen.append(ctx.request.url)

        async with await foxhound.start_crawl(
            keep_crawl,
            seeds=[seed("/relative-only", keep_page)],
            settings={"crawl": {"keep_alive": True}},
        ) as keep_run:
            async with keep_run.progress() as progress:
                first = await progress.__anext__()
                assert first.lifecycle == "running"
                assert first.idle is True
            added = await keep_run.add_seed(second_url, keep_page)
            assert added.root.kind == "added"
            keep_report = await keep_run.finish()
            assert keep_report.lifecycle == "finished"
            assert seen == [second_url]

        # 4. Keep-alive idle time counts toward `timeout.run`.
        timeout_settings = _fast_site_settings(proving_ground)
        timeout_settings["timeout"] = {"run": timedelta(milliseconds=600)}
        async with await AsyncFoxhound.connect(
            env=env, settings=timeout_settings
        ) as timeout_foxhound:
            timeout_crawl = AsyncCrawl()
            timeout_page = timeout_crawl.label("page")

            @timeout_crawl.handle(timeout_page)
            async def _on_timeout(_ctx: AsyncCrawlContext[None]) -> None:
                return None

            async with await timeout_foxhound.start_crawl(
                timeout_crawl,
                seeds=[seed(root_url, timeout_page)],
                settings={"crawl": {"keep_alive": True}},
            ) as timeout_run:
                saw_idle = False
                last_lifecycle = ""
                last_stop_reason: str | None = None
                async for step in timeout_run.progress():
                    if step.idle:
                        saw_idle = True
                    last_lifecycle = str(step.lifecycle)
                    last_stop_reason = (
                        None
                        if step.stop_reason is None
                        else str(step.stop_reason.root.kind)
                    )
                assert saw_idle is True
                assert last_lifecycle == "stopped"
                assert last_stop_reason == "timeout"

        # 5. Attempt-staged `ctx.run.add_seed` & `ctx.run.add_seeds`.
        attempts = 0
        handled_children: list[str] = []
        stage_crawl = AsyncCrawl()
        root = stage_crawl.label("root")
        child = stage_crawl.label("child")

        @stage_crawl.handle(root)
        async def _on_root(ctx: AsyncCrawlContext[None]) -> None:
            nonlocal attempts
            attempts += 1
            if attempts == 1:
                ans = await ctx.run.add_seed(discarded_single, child)
                assert ans.root.kind == "added"
                streamed = [
                    item
                    async for item in ctx.run.add_seeds([seed(discarded_stream, child)])
                ]
                assert len(streamed) == 1
                assert streamed[0].root.kind == "added"
                raise RuntimeError("intentional attempt 1 failure")
            ans = await ctx.run.add_seed(committed_single, child)
            assert ans.root.kind == "added"
            streamed = [
                item
                async for item in ctx.run.add_seeds([seed(committed_stream, child)])
            ]
            assert len(streamed) == 1
            assert streamed[0].root.kind == "added"

        @stage_crawl.handle(child)
        async def _on_child(ctx: AsyncCrawlContext[None]) -> None:
            handled_children.append(ctx.request.url)

        async with await foxhound.start_crawl(
            stage_crawl,
            seeds=[seed(root_url, root)],
            settings={"crawl": {"retry": {"max": 2, "handler_failed": True}}},
        ) as stage_run:
            stage_report = await stage_run.finish()
            assert stage_report.lifecycle == "finished"
        assert sorted(handled_children) == sorted([committed_single, committed_stream])

        # 6. `add_seeds` of 100 seeds from an async generator + `payload_invalid`.
        handled_numbers: list[int] = []
        batch_crawl = AsyncCrawl()
        shelf = batch_crawl.label("shelf", payload=Shelf)
        raw = batch_crawl.label(
            "raw",
            payload={
                "type": "object",
                "properties": {"code": {"type": "string"}},
                "required": ["code"],
                "additionalProperties": False,
            },
        )

        @batch_crawl.handle(shelf)
        async def _on_shelf(ctx: AsyncCrawlContext[Shelf]) -> None:
            handled_numbers.append(ctx.request.payload.number)

        @batch_crawl.handle(raw)
        async def _on_raw(_ctx: AsyncCrawlContext[JSONObject]) -> None:
            return None

        async with await foxhound.start_crawl(
            batch_crawl,
            seeds=[seed(root_url, shelf, Shelf(number=1))],
            settings={"crawl": {"keep_alive": True, "payload": {"max_size": 64}}},
        ) as batch_run:
            # Non-JSON value (`float("nan")`) -> `payload_invalid`.
            with pytest.raises(PayloadInvalidError):
                await batch_run.add_seed(
                    proving_ground.url(HOST, "/corpus/article/nan"),
                    raw,
                    {"code": float("nan")},
                )

            # Engine JSON Schema violation -> `payload_invalid` carrying schema issues.
            with pytest.raises(PayloadInvalidError) as schema_exc:
                await batch_run.add_seed(
                    proving_ground.url(HOST, "/corpus/article/bad-schema"),
                    raw,
                    {"code": 42},
                )
            assert schema_exc.value.typed_details is not None
            assert schema_exc.value.typed_details.issues is not None
            assert len(schema_exc.value.typed_details.issues) > 0

            # Exceeding `crawl.payload.max_size` -> `payload_invalid`.
            with pytest.raises(PayloadInvalidError):
                await batch_run.add_seed(
                    proving_ground.url(HOST, "/corpus/article/too-big"),
                    raw,
                    {"code": "x" * 120},
                )

            async def _gen_100() -> AsyncIterator[RequestSeed]:
                for i in range(2, 102):
                    yield seed(
                        proving_ground.url(HOST, f"/corpus/article/1?seed={i}"),
                        shelf,
                        Shelf(number=i),
                    )

            answers: list[Enqueued] = [
                ans async for ans in batch_run.add_seeds(_gen_100())
            ]
            assert len(answers) == 100
            for idx, ans in enumerate(answers):
                assert isinstance(ans.root, Enqueued1)
                assert ans.root.kind == "added"
                assert ans.root.url.endswith(f"?seed={idx + 2}")

            batch_report = await batch_run.finish()
            assert batch_report.lifecycle == "finished"
        assert len(handled_numbers) == 101


@pytest.mark.covers(
    "story:6-208",
    "story:6-209",
    "story:6-210",
    "operation:run.seeds",
    "operation:run.add_seed",
    "operation:run.add_seeds",
    "setting:crawl.keep_alive",
)
def test_seeds_and_add_seeds_sync(
    env: dict[str, str], proving_ground: ProvingGround
) -> None:
    """Sync `Foxhound` twin: `seeds()`, `add_seed()`, `add_seeds()`."""
    root_url = proving_ground.url(HOST, "/corpus/article/1")
    seen: list[str] = []
    crawl = Crawl()
    page = crawl.label("page")

    @crawl.handle(page)
    def _on_page(ctx: CrawlContext[None]) -> None:
        seen.append(ctx.request.url)

    with Foxhound.connect(
        env=env, settings=_fast_site_settings(proving_ground)
    ) as foxhound:
        with foxhound.start_crawl(
            crawl,
            seeds=[seed(root_url, page)],
            settings={"crawl": {"keep_alive": True}},
        ) as run:
            assert len(run.seeds()) == 1
            assert run.seeds()[0].root.kind == "added"
            one = run.add_seed(
                proving_ground.url(HOST, "/corpus/article/1?sync=1"), page
            )
            assert one.root.kind == "added"
            streamed = list(
                run.add_seeds(
                    seed(proving_ground.url(HOST, f"/corpus/article/1?sync={i}"), page)
                    for i in range(2, 6)
                )
            )
            assert len(streamed) == 4
            assert all(item.root.kind == "added" for item in streamed)
            report = run.finish()
            assert report.lifecycle == "finished"
    assert len(seen) == 6
