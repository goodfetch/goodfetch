"""An Export started from a program on Foxhound's executable (issue 1220).

A program crawls one page of the Proving ground on an Attached engine of
Foxhound's executable of the same commit, pushing one Record, then starts
an Export of its Dataset in each format the engine writes with
`run.export`, and fetches each by its handle with `export.save_as`, which
an Attached engine writes at the program's path (ADRs 0103 and 0116). The
file saved is the Export's size and SHA-256.
"""

from __future__ import annotations

import hashlib
from typing import TYPE_CHECKING, cast

import anyio
import anyio.to_thread
import pytest
from goodfetch_foxhound import AsyncCrawl, AsyncFoxhound, seed
from goodfetch_test_support import Product as ToolProduct

if TYPE_CHECKING:
    from collections.abc import Mapping
    from pathlib import Path

    from goodfetch_collie_client.json_values import JSONObject
    from goodfetch_foxhound import AsyncCrawlContext
    from goodfetch_foxhound.generated.settings import CrawlSettings
    from goodfetch_test_support import ProvingGround

TASK = "extraction/7"
"""The Proving ground's task the crawl reads one page of."""


def settings(proving_ground: ProvingGround) -> CrawlSettings:
    """The Run's settings: plain HTTP fetches of the Proving ground's hosts.

    Each host maps to its address, which the run may reach although it is
    not globally reachable.
    """
    resolve = {host: [address] for host, address in proving_ground.resolve.items()}
    return {
        "fetch": {"tier": "http"},
        "network": {"resolve": resolve},
        "egress": {"private_ranges": list(resolve)},
    }


FORMATS = (
    "json",
    "jsonl",
    "csv",
    "xlsx",
    "xml",
    "parquet",
    "html_table",
    "rss",
    "atom",
    "avro",
    "arrow_ipc",
    "markdown",
)
"""The formats the engine writes an Export in."""


@pytest.fixture
def product() -> ToolProduct:
    """The test runs Foxhound's executable."""
    return ToolProduct.FOXHOUND


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground's own seed for this test."""
    return 1220


@pytest.mark.covers("story:12-213", "operation:run.export", "operation:export.save_as")
@pytest.mark.anyio
async def test_an_export_starts_in_each_format_and_is_fetched_by_handle(
    env: Mapping[str, str], proving_ground: ProvingGround, tmp_path: Path
) -> None:
    """Each format's Export, saved by its handle, is the file the engine wrote."""
    start = await anyio.to_thread.run_sync(proving_ground.start_url, TASK)
    crawl = AsyncCrawl()
    page = crawl.label("page")
    pages = crawl.dataset("pages")

    @crawl.handle(page)
    async def push_one(ctx: AsyncCrawlContext[None]) -> None:
        record: JSONObject = {"page": 1, "title": "Café"}
        await ctx.push(pages, record)

    foxhound = await AsyncFoxhound.connect(env=env)
    run = await foxhound.start_crawl(
        crawl,
        seeds=[seed(start, page)],
        settings=settings(proving_ground),
        run_id="exported",
    )
    report = await run.finish()
    fetched: list[tuple[str, object, bool]] = []
    for format_ in FORMATS:
        export = cast(
            "JSONObject",
            await foxhound.call(
                "run.export",
                {"run_id": "exported", "dataset": "pages", "format": format_},
            ),
        )
        path = tmp_path / f"pages.{format_}"
        saved = cast(
            "JSONObject",
            await foxhound.call(
                "export.save_as", {"export_id": export["export_id"], "path": str(path)}
            ),
        )
        written = path.read_bytes()
        digest = hashlib.sha256(written).hexdigest()
        fetched.append(
            (
                format_,
                export["records"],
                (saved["size"], saved["sha256"])
                == (export["size"], export["sha256"])
                == (len(written), digest),
            )
        )
    await foxhound.close()

    assert report.lifecycle == "finished"
    assert fetched == [(format_, 1, True) for format_ in FORMATS]
