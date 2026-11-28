"""Dataset Exports through `goodfetch_foxhound` on an Attached engine (issue 1488).

Tests `AsyncCrawlRun.export` and sync `CrawlRun.export`, `save_as`, `stream`,
CSV BOM toggling via `export.csv.bom`, UTF-8 round-tripping, and
`PathNotWritableError` on an unwritable path leaving no partial file.
"""

from __future__ import annotations

import hashlib
from typing import TYPE_CHECKING

import anyio
import anyio.to_thread
import pytest
from goodfetch_collie_client.generated.errors import PathNotWritableError
from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncFoxhound,
    Crawl,
    ExportFormat,
    Foxhound,
    seed,
)
from goodfetch_test_support import Product as ToolProduct

if TYPE_CHECKING:
    from collections.abc import Mapping
    from pathlib import Path

    from goodfetch_collie_client.json_values import JSONObject
    from goodfetch_foxhound import AsyncCrawlContext, CrawlContext
    from goodfetch_foxhound.generated.settings import CrawlSettings
    from goodfetch_test_support import ProvingGround

TASK = "extraction/7"
"""Proving ground task with one page to crawl."""

UTF8_BOM = b"\xef\xbb\xbf"
"""UTF-8 Byte Order Mark."""

FORMATS: tuple[ExportFormat, ...] = (
    ExportFormat.json,
    ExportFormat.jsonl,
    ExportFormat.csv,
    ExportFormat.xlsx,
    ExportFormat.xml,
    ExportFormat.parquet,
    ExportFormat.html_table,
    ExportFormat.rss,
    ExportFormat.atom,
    ExportFormat.avro,
    ExportFormat.arrow_ipc,
    ExportFormat.markdown,
)
"""Every Export format `run.export` supports."""


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
    return 1488


@pytest.mark.covers(
    "story:6-276",
    "story:6-277",
    "operation:run.export",
    "operation:export.save_as",
    "operation:export.stream",
)
@pytest.mark.anyio
async def test_async_export_all_formats_bom_utf8_and_unwritable_path(
    env: Mapping[str, str], proving_ground: ProvingGround, tmp_path: Path
) -> None:
    """Async `export`, `save_as`, and `stream` match across all 12 formats."""
    start = await anyio.to_thread.run_sync(proving_ground.start_url, TASK)
    crawl = AsyncCrawl()
    page = crawl.label("page")
    notes = crawl.dataset("notes")

    @crawl.handle(page)
    async def push_one(ctx: AsyncCrawlContext[None]) -> None:
        record: JSONObject = {"title": "Café — 日本語", "count": 1}
        await ctx.push(notes, record)

    foxhound = await AsyncFoxhound.connect(env=env)
    try:
        run = await foxhound.start_crawl(
            crawl,
            seeds=[seed(start, page)],
            settings=site_settings(proving_ground),
            run_id="py-async-export",
        )
        report = await run.finish()
        assert report.lifecycle == "finished"

        for fmt in FORMATS:
            exported = await run.export(notes, format=fmt)
            assert exported.format == fmt
            assert exported.records == 1
            assert exported.last_number == 1

            path = tmp_path / fmt.value / "nested" / f"notes.{fmt.value}"
            saved = await exported.save_as(path)
            file_bytes = path.read_bytes()
            streamed = b"".join([chunk async for chunk in exported.stream()])
            digest = hashlib.sha256(file_bytes).hexdigest()

            assert exported.size == len(file_bytes)
            assert exported.sha256 == digest
            assert saved.size == exported.size
            assert saved.sha256 == exported.sha256
            assert streamed == file_bytes

            if fmt is ExportFormat.csv:
                assert not file_bytes.startswith(UTF8_BOM)
                assert "Café — 日本語" in file_bytes.decode("utf-8")

        csv_path = tmp_path / "csv" / "nested" / "notes.csv"
        csv_bom = await run.export(
            notes,
            format=ExportFormat.csv,
            settings={"export": {"csv": {"bom": True}}},
        )
        await csv_bom.save_as(csv_path)
        bom_bytes = csv_path.read_bytes()
        assert bom_bytes.startswith(UTF8_BOM)
        assert "Café — 日本語" in bom_bytes[3:].decode("utf-8")

        dir_target = tmp_path / "existing-dir"
        dir_target.mkdir()
        with pytest.raises(PathNotWritableError) as exc_info:
            await csv_bom.save_as(dir_target)
        assert exc_info.value.retryable is False
        assert not _partial_files(tmp_path)
    finally:
        await foxhound.close()


def _partial_files(root: Path) -> list[Path]:
    return [p for p in root.iterdir() if ".partial" in p.name]


@pytest.mark.covers(
    "story:6-276",
    "story:6-277",
    "operation:run.export",
    "operation:export.save_as",
    "operation:export.stream",
)
def test_sync_export_save_as_and_stream_match(
    env: Mapping[str, str], proving_ground: ProvingGround, tmp_path: Path
) -> None:
    """Sync `CrawlRun.export`, `save_as`, and `stream` match."""
    start = proving_ground.start_url(TASK)
    crawl = Crawl()
    page = crawl.label("page")
    notes = crawl.dataset("notes")

    @crawl.handle(page)
    def push_one(ctx: CrawlContext[None]) -> None:
        record: JSONObject = {"title": "Sync Café", "count": 1}
        ctx.push(notes, record)

    with Foxhound.connect(env=env) as foxhound:
        run = foxhound.start_crawl(
            crawl,
            seeds=[seed(start, page)],
            settings=site_settings(proving_ground),
            run_id="py-sync-export",
        )
        report = run.finish()
        assert report.lifecycle == "finished"

        exported = run.export(notes, format="jsonl")
        path = tmp_path / "sync" / "notes.jsonl"
        saved = exported.save_as(path)
        file_bytes = path.read_bytes()
        streamed = b"".join(exported.stream())

        assert exported.records == 1
        assert exported.size == len(file_bytes)
        assert saved.sha256 == exported.sha256
        assert streamed == file_bytes
        assert "Sync Café" in file_bytes.decode("utf-8")
