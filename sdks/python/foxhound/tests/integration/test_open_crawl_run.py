"""Reading a crawl Run another program started: `open_crawl_run` (ADRs 0102, 0103).

Tests `AsyncFoxhound.open_crawl_run` and `Foxhound.open_crawl_run` (issue 1489):
live typed Dataset reads, clean end when the writer is gone, `RunInterruptedError`
with a fix naming the Run ID on `report()`, no writer lock blocking resume,
and `outcomes`, `failed_requests`, `rejected_records`, `export`, `progress`,
`key_value_store`, and `close()`.
"""

from __future__ import annotations

import shutil
from pathlib import Path
from typing import TYPE_CHECKING

import anyio
import anyio.to_thread
import pytest
from goodfetch_collie_client.generated.errors import ClosedError, RunInterruptedError
from goodfetch_foxhound import AsyncCrawl, AsyncFoxhound, Crawl, Foxhound, seed
from goodfetch_test_support import Product as ToolProduct
from pydantic import BaseModel

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_foxhound import AsyncCrawlContext
    from goodfetch_foxhound.generated.settings import CrawlSettings
    from goodfetch_test_support import ProvingGround


def _find_run_sqlite(home: Path, run_id: str) -> Path:
    for entry in home.iterdir():
        candidate = entry / "runs" / run_id / "run.sqlite"
        if candidate.exists():
            return candidate
    direct = home / "runs" / run_id / "run.sqlite"
    if direct.exists():
        return direct
    msg = f"run.sqlite for {run_id} not found under {home}"
    raise FileNotFoundError(msg)


def _restore_run_file(src: Path, dst: Path) -> None:
    for suffix in ("-wal", "-shm"):
        sidecar = Path(f"{dst}{suffix}")
        if sidecar.exists():
            sidecar.unlink()
    shutil.copyfile(src, dst)


class Item(BaseModel):
    """One crawled item."""

    id: str
    title: str


def _site_settings(proving_ground: ProvingGround) -> CrawlSettings:
    resolve = {host: [address] for host, address in proving_ground.resolve.items()}
    return {
        "fetch": {"tier": "http"},
        "network": {"resolve": resolve},
        "egress": {"private_ranges": list(resolve.keys())},
    }


@pytest.fixture
def product() -> ToolProduct:
    """The test runs Foxhound's executable."""
    return ToolProduct.FOXHOUND


@pytest.fixture
def proving_ground_seed() -> int:
    """Seed for the Proving ground."""
    return 1489


@pytest.mark.covers("story:11-1068")
@pytest.mark.anyio
async def test_open_crawl_run_live_interrupted_and_resume(
    env: Mapping[str, str], proving_ground: ProvingGround, tmp_path: Path
) -> None:
    """A second program reads live Records and never blocks resume."""
    first_url = await anyio.to_thread.run_sync(proving_ground.start_url, "catalog/1")
    run_id = "py-open-crawl-run-live"

    first_committed = anyio.Event()

    writer_crawl = AsyncCrawl()
    page = writer_crawl.label("page")
    items = writer_crawl.dataset("items", schema=Item, key=["id"])

    @writer_crawl.handle(page)
    async def handle(ctx: AsyncCrawlContext[None]) -> None:
        await ctx.push(
            items,
            [Item(id="item-1", title="First"), Item(id="item-2", title="Second")],
        )
        await ctx.run.key_value_store.set("stage", "one")
        first_committed.set()

    settings: CrawlSettings = {
        **_site_settings(proving_ground),
        "crawl": {"handler": {"concurrency": 1}, "keep_alive": True},
    }

    async with (
        AsyncFoxhound.connect(env=env) as writer,
        AsyncFoxhound.connect(env=env) as reader,
    ):
        run = await writer.start_crawl(
            writer_crawl,
            run_id=run_id,
            seeds=[seed(first_url, page)],
            settings=settings,
        )
        await first_committed.wait()

        # Second program declares its own Dataset handle and opens a read-only view.
        reader_crawl = AsyncCrawl()
        reader_items = reader_crawl.dataset("items", schema=Item, key=["id"])
        view = await reader.open_crawl_run(run_id)
        assert view.id == run_id

        # Read the first committed Record live.
        checkpoint_path = tmp_path / "interrupted.sqlite"
        async with view.records(reader_items) as live_records:
            first_rec = await anext(live_records)
            assert first_rec.number == 1
            assert first_rec.value == Item(id="item-1", title="First")

            # Snapshot the running Run before stopping the writer.
            checkpoint = await run.checkpoint()
            await checkpoint.save_as(checkpoint_path)

            await run.stop()
            await writer.close()

            # The live stream yields item-2 then ends cleanly after the last Record.
            second_rec = await anext(live_records)
            assert second_rec.number == 2
            with pytest.raises(StopAsyncIteration):
                await anext(live_records)

        # Restore running snapshot so the Run is interrupted (no live writer).
        run_sqlite = _find_run_sqlite(Path(env["GOODFETCH_HOME"]), run_id)
        _restore_run_file(checkpoint_path, run_sqlite)

        async with view.records(reader_items) as interrupted_seq:
            interrupted_recs = [r async for r in interrupted_seq]
        assert len(interrupted_recs) == 2
        assert interrupted_recs[0].value == Item(id="item-1", title="First")

        # Interrupted Run: `report()` raises `RunInterruptedError` naming run_id.
        with pytest.raises(RunInterruptedError) as exc_info:
            await view.report()
        assert exc_info.value.fix is not None
        assert run_id in exc_info.value.fix

        # Key-value store and progress read without taking a writer lock.
        assert await view.key_value_store.get("stage") == "one"
        async with view.progress() as prog:
            progress_items = [p async for p in prog]
        assert len(progress_items) > 0

        # Holding the view takes no writer lock: a third connection resumes the Run.
        resume_crawl = AsyncCrawl()
        resume_page = resume_crawl.label("page")
        resume_crawl.dataset("items", schema=Item, key=["id"])

        @resume_crawl.handle(resume_page)
        async def handle_resume(_ctx: AsyncCrawlContext[None]) -> None:
            return None

        async with AsyncFoxhound.connect(env=env) as resumer:
            resumed = await resumer.start_crawl(
                resume_crawl,
                run_id=run_id,
                seeds=[seed(first_url, resume_page)],
                settings=settings,
            )
            assert resumed.started_as == "resumed"
            await resumed.finish()

        # Run ended: view reads report, records(after), export, outcomes, failed.
        report = await view.report()
        assert report.run_id.root == run_id

        async with view.records(reader_items, after=1) as rec_after:
            after_one = [r async for r in rec_after]
        assert len(after_one) == 1
        assert after_one[0].number == 2
        assert after_one[0].value == Item(id="item-2", title="Second")

        exported = await view.export(reader_items, format="json")
        assert exported.records == 2

        async with view.outcomes() as out_seq:
            outcomes = [o async for o in out_seq]
        assert len(outcomes) == 1

        failed = await view.failed_requests()
        assert failed == []

        await view.close()
        with pytest.raises(ClosedError):
            await view.report()

    # Sync API check on the completed Run.
    def _check_sync() -> None:
        sync_crawl = Crawl()
        sync_items = sync_crawl.dataset("items", schema=Item, key=["id"])
        with Foxhound.connect(env=env) as sync_reader:
            with sync_reader.open_crawl_run(run_id) as sync_view:
                assert sync_view.id == run_id
                with sync_view.records(sync_items, after=1) as seq:
                    recs = list(seq)
                assert len(recs) == 1
                assert recs[0].value == Item(id="item-2", title="Second")
                assert sync_view.export(sync_items, format="json").records == 2
                assert sync_view.failed_requests() == []
                assert sync_view.report().run_id.root == run_id

    await anyio.to_thread.run_sync(_check_sync)
