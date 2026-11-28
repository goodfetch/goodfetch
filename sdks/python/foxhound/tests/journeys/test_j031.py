"""J-031 on the `sdk-python` surface (ADRs 0063, 0075 and 0086; issue 688).

"Crawl with robots.txt turned off and find it in the Run report, the Audit log
and the Records' Provenance": runs a crawl through `goodfetch_foxhound` over an
allowed and a disallowed page of `1.access-policy.test` with
`site_preferences.respect_robots_txt` off and
`site_preferences.mark_robots_txt_ignored` on, reads the Run report, exports
the Audit log, and checks the Records' Provenance.
"""

from __future__ import annotations

import json
from typing import TYPE_CHECKING, Any, cast
from urllib.parse import urlparse

import anyio.to_thread
import pytest
from goodfetch_collie_client.generated.models import Lifecycle
from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncCrawlContext,
    AsyncFoxhound,
    DatasetRecord,
    seed,
)
from goodfetch_test_support import JourneyRun, journey

if TYPE_CHECKING:
    from goodfetch_foxhound import AsyncCrawlRun
    from goodfetch_foxhound.generated.models import CrawlRunReport
    from goodfetch_foxhound.generated.settings import CrawlSettings
    from goodfetch_test_support import ProvingGround

HOST = "1.access-policy.test"
RUN_ID = "robots-txt-off"


def _settings_for(proving_ground: ProvingGround) -> CrawlSettings:
    ip = proving_ground.resolve[HOST]
    return {
        "fetch": {"tier": "http"},
        "network": {"resolve": {HOST: [ip]}},
        "egress": {"private_ranges": [HOST]},
        "site_preferences": {
            "respect_robots_txt": False,
            "mark_robots_txt_ignored": True,
        },
    }


@pytest.mark.covers(
    "story:4-117",
    "story:4-118",
    "row:SC-01",
    "row:compliance-site-owner-politeness",
)
@journey("J-031", "sdk-python")
async def test_j031(run: JourneyRun) -> None:
    """J-031 on `sdk-python`."""
    proving_ground = run.proving_ground
    assert proving_ground is not None, "J-031 has a Proving ground"

    crawl = AsyncCrawl()
    page = crawl.label("page")
    pages = crawl.dataset("pages")

    @crawl.handle(page)
    async def handle_page(context: AsyncCrawlContext[None]) -> None:
        item: dict[str, Any] = {"url": context.request.url}
        await context.push(pages, item)

    foxhound: AsyncFoxhound | None = None
    crawl_run: AsyncCrawlRun | None = None
    report: CrawlRunReport | None = None
    exported: list[dict[str, Any]] = []
    records: list[DatasetRecord[dict[str, Any]]] = []

    try:

        async def run_crawl_step() -> None:
            nonlocal foxhound, crawl_run, report
            foxhound = await AsyncFoxhound.connect(env=run.env)
            crawl_run = await foxhound.start_crawl(
                crawl,
                run_id=RUN_ID,
                seeds=[
                    seed(proving_ground.url(HOST, "/catalog/1"), page),
                    seed(proving_ground.url(HOST, "/private/catalog/1"), page),
                ],
                settings=_settings_for(proving_ground),
            )
            report = await crawl_run.finish()

        await run.step(
            "Run a crawl from a program over a page the site's robots.txt allows "
            "and one it disallows, with `site_preferences.respect_robots_txt` "
            "turned off and `site_preferences.mark_robots_txt_ignored` turned on "
            "for the run, each page's handler pushing a Record",
            run_crawl_step,
        )

        async def read_report_step() -> None:
            assert report is not None
            assert report.lifecycle == Lifecycle.finished

        await run.step("Read the run's report", read_report_step)

        def export_audit_sync() -> list[dict[str, Any]]:
            proc = run.command(["audit", "export"])
            stdout, stderr = proc.communicate()
            assert proc.returncode == 0, stderr.decode(errors="replace")
            return [
                cast("dict[str, Any]", json.loads(line))
                for line in stdout.decode("utf-8").splitlines()
                if line
            ]

        async def export_audit_step() -> None:
            exported.extend(await anyio.to_thread.run_sync(export_audit_sync))

        await run.step("Export the Audit log", export_audit_step)

        async def read_records_step() -> None:
            assert crawl_run is not None
            async with crawl_run.records(pages) as stream:
                records.extend(
                    [
                        cast("DatasetRecord[dict[str, Any]]", record)
                        async for record in stream
                    ]
                )

        await run.step("Read the crawl's Records", read_records_step)
    finally:
        if foxhound is not None:
            await foxhound.close()

    async def check_report() -> None:
        assert report is not None
        listed = [
            (entry.value, str(entry.layer))
            for entry in (report.weakened or [])
            if entry.key == "site_preferences.respect_robots_txt"
        ]
        assert listed == [(False, "run")], report

    await run.check(
        "The Run report lists `site_preferences.respect_robots_txt` in "
        "`weakened`, set by the run",
        check_report,
    )

    async def check_audit() -> None:
        entries = [
            (entry.get("run_id"), entry.get("layer"), entry.get("details"))
            for entry in exported
            if entry.get("kind") == "politeness_weakened"
        ]
        assert entries == [
            (
                RUN_ID,
                "run",
                {
                    "setting": "site_preferences.respect_robots_txt",
                    "value": False,
                    "default": True,
                },
            )
        ], exported

    await run.check(
        "The Audit log holds one `politeness_weakened` entry for the run, naming "
        "`site_preferences.respect_robots_txt`, its value `false`, its built-in "
        "default `true` and the run's layer",
        check_audit,
    )

    async def check_records() -> None:
        marks = sorted(
            (
                urlparse(cast("str", record.value["url"])).path,
                None
                if record.provenance.robots_txt is None
                else str(record.provenance.robots_txt),
            )
            for record in records
        )
        assert marks == [
            ("/catalog/1", None),
            ("/private/catalog/1", "ignored"),
        ], records

    await run.check(
        'The Record of the disallowed page carries `robots_txt: "ignored"` in '
        "its Provenance, and the Record of the allowed page carries no mark",
        check_records,
    )
