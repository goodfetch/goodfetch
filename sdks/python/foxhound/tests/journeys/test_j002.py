"""J-002 on the `sdk-python` surface (ADRs 0063, 0069, 0075 and 0094; issue 141).

"Crawl a site over HTTP to Markdown and export its Dataset as JSON, CSV and
Parquet": lays out Foxhound's `py3-none` platform wheel in a fresh site folder
with no install scripts, verifies the commit's Product version across
executable and SDK packages, crawls `catalog/1` over HTTP to Markdown with
`CrawlExtraction`, streams Records as they arrive, and exports JSON, CSV and
Parquet.
"""

from __future__ import annotations

import csv
import datetime
import importlib.metadata
import io
import json
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import TYPE_CHECKING, Any, cast

import anyio
import anyio.to_thread
import goodfetch_foxhound
import pytest
from goodfetch_collie_client.generated.models import Lifecycle
from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncFoxhound,
    CrawlExtraction,
    DatasetRecord,
    ExportFormat,
)
from goodfetch_foxhound.generated.models import RequestSeed
from goodfetch_test_support import JourneyRun, journey

if TYPE_CHECKING:
    from goodfetch_foxhound import AsyncCrawlRun
    from goodfetch_foxhound.generated.models import CrawlRequestOutcome
    from goodfetch_foxhound.generated.settings import CrawlSettings
    from goodfetch_test_support import ProvingGround

PACKAGE_ROOT = Path(__file__).parents[2]
"""Root of `sdks/python/foxhound`."""

EXPORT_FORMATS: tuple[tuple[str, ExportFormat], ...] = (
    ("json", ExportFormat.json),
    ("csv", ExportFormat.csv),
    ("parquet", ExportFormat.parquet),
)


def _site_settings(proving_ground: ProvingGround) -> CrawlSettings:
    """Plain HTTP settings resolving the Proving ground's hosts."""
    resolve = {host: [address] for host, address in proving_ground.resolve.items()}
    return {
        "fetch": {"tier": "http"},
        "politeness": {"domain": {"delay": datetime.timedelta(0)}},
        "network": {"resolve": resolve},
        "egress": {"private_ranges": list(resolve)},
    }


def _install_fresh_wheel(site: Path, engine: Path) -> Path:
    """Lays out Foxhound's platform wheel in `site` with `engine` under `bin/`."""
    package = site / goodfetch_foxhound.__name__
    shutil.copytree(
        Path(goodfetch_foxhound.__file__).parent,
        package,
        ignore=shutil.ignore_patterns("__pycache__", "bin"),
    )
    suffix = ".exe" if sys.platform == "win32" else ""
    executable = package / "bin" / f"goodfetch-foxhound{suffix}"
    executable.parent.mkdir()
    shutil.copy2(engine, executable)
    return executable


@pytest.mark.covers(
    "story:1-1",
    "story:1-2",
    "story:1-3",
    "story:11-1130",
    "story:11-1132",
)
@journey("J-002", "sdk-python")
async def test_j002(run: JourneyRun) -> None:
    """J-002 on `sdk-python`."""
    proving_ground = run.proving_ground
    assert proving_ground is not None, "J-002 has a Proving ground"

    pyproject = tomllib.loads((PACKAGE_ROOT / "pyproject.toml").read_text("utf-8"))
    project_meta = cast("dict[str, Any]", pyproject["project"])
    sdk_version = cast("str", project_meta["version"])
    run.surface_package("goodfetch-foxhound", sdk_version)

    executable_version = ""
    foxhound: AsyncFoxhound | None = None
    crawl_run: AsyncCrawlRun | None = None
    followed: list[DatasetRecord[dict[str, Any]]] = []
    committed: list[DatasetRecord[dict[str, Any]]] = []
    outcomes: list[CrawlRequestOutcome] = []
    exports_meta: list[tuple[str, Path, int, int]] = []

    schema_crawl = AsyncCrawl()
    pages = schema_crawl.dataset("pages")

    try:

        def install_step() -> None:
            nonlocal executable_version
            site = run.test_env.folder.resolve() / "fresh-site"
            executable = _install_fresh_wheel(site, run.engine)
            completed = subprocess.run(
                [str(executable), "--version"],
                capture_output=True,
                text=True,
                check=True,
            )
            executable_version = completed.stdout.strip()

        await run.step(
            "Install Foxhound into a fresh project from the commit's packages alone, "
            "with install scripts ignored: the surface's SDK package, or Foxhound's "
            "executable on the other surfaces",
            lambda: anyio.to_thread.run_sync(install_step),
        )

        async def crawl_step() -> None:
            nonlocal foxhound, crawl_run
            start = await anyio.to_thread.run_sync(
                proving_ground.start_url, "catalog/1"
            )
            extraction = CrawlExtraction.model_validate(
                {
                    "formats": {"markdown": True},
                    "follow": "scope",
                    "fields": {
                        "id": {
                            "locator": {
                                "kind": "css",
                                "selector": "article.product .id",
                            }
                        },
                        "name": {
                            "locator": {
                                "kind": "css",
                                "selector": "article.product .name",
                            }
                        },
                        "price": {
                            "locator": {
                                "kind": "css",
                                "selector": "article.product .price",
                            },
                            "attribute": "data-price",
                        },
                    },
                }
            )
            foxhound = await AsyncFoxhound.connect(env=run.env)
            crawl_run = await foxhound.start_crawl(
                extraction,
                run_id="catalog-py",
                seeds=[RequestSeed(url=start, label="page")],
                settings=_site_settings(proving_ground),
            )

        await run.step(
            "Crawl the catalog over HTTP with no browser, from its first listing "
            "page, turning each page into Markdown",
            crawl_step,
        )

        async def read_step() -> None:
            assert crawl_run is not None, "crawl started"
            active_run = crawl_run
            report_lifecycle: list[Lifecycle] = []

            async def collect_records() -> None:
                async with active_run.records(pages) as stream:
                    followed.extend(
                        [
                            cast("DatasetRecord[dict[str, Any]]", record)
                            async for record in stream
                        ]
                    )

            async def collect_outcomes() -> None:
                async with active_run.outcomes() as stream:
                    outcomes.extend([outcome async for outcome in stream])

            async def await_finish() -> None:
                report = await active_run.finish()
                report_lifecycle.append(report.lifecycle)

            async with anyio.create_task_group() as group:
                group.start_soon(collect_records)
                group.start_soon(collect_outcomes)
                group.start_soon(await_finish)

            assert report_lifecycle == [Lifecycle.finished]

        await run.step("Read the crawl's Records as they arrive", read_step)

        async def export_step() -> None:
            assert crawl_run is not None, "crawl started"
            for name, fmt in EXPORT_FORMATS:
                exported = await crawl_run.export(pages, format=fmt)
                out = run.test_env.folder / f"catalog.{name}"
                await exported.save_as(out)
                exports_meta.append((name, out, exported.size, exported.records))
            async with crawl_run.records(pages) as stream:
                committed.extend(
                    [
                        cast("DatasetRecord[dict[str, Any]]", record)
                        async for record in stream
                    ]
                )

        await run.step(
            "Export the crawl's Dataset as JSON, as CSV and as Parquet",
            export_step,
        )
    finally:
        if foxhound is not None:
            await foxhound.close()

    async def check_version() -> None:
        assert executable_version == f"goodfetch-foxhound {sdk_version}"
        assert importlib.metadata.version("goodfetch-foxhound") == sdk_version
        assert importlib.metadata.version("goodfetch-collie-client") == sdk_version

    await run.check(
        "The installed package runs, and reports the commit's Product version, "
        "the one Foxhound's executable and every SDK package report",
        check_version,
    )

    async def check_no_install_script() -> None:
        build_system = cast("dict[str, Any]", pyproject.get("build-system", {}))
        assert build_system.get("build-backend") == "uv_build"
        assert "setup.py" not in {p.name for p in PACKAGE_ROOT.iterdir()}

    await run.check(
        "The install ran no install script and reached no registry beyond the "
        "commit's packages",
        check_no_install_script,
    )

    async def check_platform_wheel() -> None:
        assert project_meta.get("requires-python") == ">=3.11"

    await run.check(
        "The installed package is the one platform package of its language for "
        "every runtime version Foxhound supports, such as one `py3-none` platform "
        "wheel for every supported CPython",
        check_platform_wheel,
    )

    products = [
        {
            "id": record.value["id"],
            "name": record.value["name"],
            "price": record.value["price"],
        }
        for record in committed
        if isinstance(record.value.get("id"), str)
    ]
    await run.expect_task(
        "The oracle verifies the crawl's Records",
        {"status": "success", "records": products},
    )

    async def check_http_only() -> None:
        tiers = {
            record.provenance.tier.tier
            for record in committed
            if record.provenance is not None and record.provenance.tier is not None
        }
        kinds = {outcome.root.kind for outcome in outcomes}
        assert tiers == {"http"}
        assert kinds == {"succeeded"}
        assert len(committed) == len(outcomes)

    await run.check(
        "The crawl fetched every page over HTTP and launched no browser",
        check_http_only,
    )

    async def check_markdown() -> None:
        assert committed
        for record in committed:
            markdown = record.value.get("markdown")
            assert isinstance(markdown, str) and len(markdown) > 0

    await run.check("Each Record holds its page's Markdown", check_markdown)

    async def check_order() -> None:
        assert [record.number for record in followed] == list(
            range(1, len(committed) + 1)
        )
        assert [record.value for record in followed] == [
            record.value for record in committed
        ]

    await run.check(
        "The Records read as they arrived are the Dataset's, in order from its "
        "first, each once",
        check_order,
    )

    async def check_exports() -> None:
        values = [record.value for record in committed]
        fields = sorted({key for value in values for key in value})
        assert [name for name, _, _, _ in exports_meta] == ["json", "csv", "parquet"]
        for name, out, size, record_count in exports_meta:
            raw = out.read_bytes()
            assert len(raw) == size
            assert record_count == len(values)
            if name == "json":
                assert json.loads(raw.decode("utf-8")) == values
            elif name == "csv":
                reader = csv.DictReader(io.StringIO(raw.decode("utf-8")))
                assert sorted(reader.fieldnames or []) == fields
                assert sum(1 for _ in reader) == len(values)
            else:
                assert raw[:4] == b"PAR1"
                assert raw[-4:] == b"PAR1"
                latin1 = raw.decode("latin-1")
                for field in fields:
                    assert field in latin1

    await run.check(
        "The JSON, CSV and Parquet Exports each hold every Record of the Dataset, "
        "with every field",
        check_exports,
    )
