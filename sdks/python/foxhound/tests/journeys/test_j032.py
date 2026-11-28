"""J-032 on the `sdk-python` surface (ADRs 0042, 0063 and 0075; issue 688).

"Crawl a site whose certificate is invalid by listing its domain for trust on
first use, and see the crawl stop when the key changes until I accept it":
crawls `expired.certificates.test` through `goodfetch_foxhound` with
`tls.accept_invalid_certs`, verifies the crawl stops on a changed key with
`certificate_changed`, verifies `certs.accept` is refused both through the SDK
(`person_required`) and through non-interactive `--json`, accepts the new key
in a terminal (`certs accept`), and crawls again.
"""

from __future__ import annotations

import datetime
import os
import subprocess
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, cast

import anyio.to_thread
import pytest
from goodfetch_collie_client.generated.models import Lifecycle
from goodfetch_collie_client.goodfetch_error import GoodfetchError
from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncCrawlContext,
    AsyncFoxhound,
    DatasetRecord,
    seed,
)
from goodfetch_foxhound.generated.models import CertificateExceptionKind
from goodfetch_test_support import JourneyRun, journey
from goodfetch_test_support.environment import TestEnv
from goodfetch_test_support.tools import Tool

if TYPE_CHECKING:
    from collections.abc import Mapping, Sequence
    from pathlib import Path

    from goodfetch_foxhound.generated.models import CrawlRequestOutcome
    from goodfetch_foxhound.generated.settings import CrawlSettings
    from goodfetch_test_support import ProvingGround

HOST = "expired.certificates.test"
CHANGED_SEED = 1032


@dataclass(frozen=True)
class Crawled:
    """One crawl's outcome kind, error codes, pinned key, and record count."""

    kind: str
    errors: tuple[str, ...]
    pinned_key: str | None
    record_count: int


def _site_url(proving_ground: ProvingGround) -> str:
    port = proving_ground.address.rpartition(":")[2]
    return f"https://{HOST}:{port}/"


def _settings_for(proving_ground: ProvingGround) -> CrawlSettings:
    resolve = {host: [address] for host, address in proving_ground.resolve.items()}
    return {
        "fetch": {"tier": "http"},
        "politeness": {"domain": {"delay": datetime.timedelta(0)}},
        "site_preferences": {"respect_robots_txt": False},
        "network": {"resolve": resolve},
        "egress": {"private_ranges": list(resolve)},
        "tls": {"accept_invalid_certs": {"certificates.test": None}},
    }


async def _run_crawl(
    foxhound: AsyncFoxhound,
    proving_ground: ProvingGround,
    run_id: str,
) -> Crawled:
    crawl = AsyncCrawl()
    page = crawl.label("page")
    pages = crawl.dataset("pages")

    @crawl.handle(page)
    async def handle_page(context: AsyncCrawlContext[None]) -> None:
        item: dict[str, Any] = {"url": context.request.url}
        await context.push(pages, item)

    active = await foxhound.start_crawl(
        crawl,
        run_id=run_id,
        seeds=[seed(_site_url(proving_ground), page)],
        settings=_settings_for(proving_ground),
    )
    report = await active.finish()
    assert report.lifecycle == Lifecycle.finished, report

    outcomes: list[CrawlRequestOutcome] = []
    async with active.outcomes() as outcome_stream:
        outcomes.extend([outcome async for outcome in outcome_stream])
    assert outcomes, f"crawl {run_id} returned no outcomes"
    first_outcome = outcomes[0].root

    records: list[DatasetRecord[dict[str, Any]]] = []
    async with active.records(pages) as record_stream:
        records.extend(
            [
                cast("DatasetRecord[dict[str, Any]]", record)
                async for record in record_stream
            ]
        )

    pinned_key: str | None = None
    if len(records) == 1:
        mark = records[0].provenance.certificate_exception
        if (
            mark is not None
            and mark.kind == CertificateExceptionKind.trust_on_first_use
        ):
            pinned_key = mark.spki_sha256

    return Crawled(
        kind=first_outcome.kind,
        errors=tuple(error.code for error in first_outcome.errors),
        pinned_key=pinned_key,
        record_count=len(records),
    )


def _accept_cert_in_terminal(
    test_env_bin: Path,
    executable: Path,
    host: str,
    env: Mapping[str, str],
    cwd: Path,
) -> tuple[int, str]:
    """Runs `certs accept <host>` on a cross-OS PTY (`goodfetch-test-env pty`)."""
    completed = subprocess.run(
        [
            str(test_env_bin),
            "pty",
            "--expect",
            "[y/N]",
            "--send-line",
            "y",
            "--",
            str(executable),
            "certs",
            "accept",
            host,
        ],
        env=dict(env),
        cwd=cwd,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=60,
        check=False,
    )
    return completed.returncode, completed.stdout + completed.stderr


@pytest.mark.covers("story:4-35", "story:4-36", "story:4-104", "row:SC-06")
@journey("J-032", "sdk-python")
async def test_j032(run: JourneyRun) -> None:
    """Covers: story:4-35, story:4-36, story:4-104, row:SC-06."""
    proving_ground = run.proving_ground
    assert proving_ground is not None, "J-032 has a Proving ground"

    changed_env: TestEnv | None = None
    foxhound: AsyncFoxhound | None = None
    first: Crawled | None = None
    changed: Crawled | None = None
    changed_requests = -1
    sdk_accept_error_code = ""
    json_accept_exit = 0
    accepted: Crawled | None = None

    try:
        foxhound = await AsyncFoxhound.connect(env=run.env)
        client = foxhound

        async def step_first() -> None:
            nonlocal first
            first = await _run_crawl(client, proving_ground, "first-use")

        await run.step(
            "Crawl the site from a program, listing its domain for trust on "
            "first use in the crawl's settings",
            step_first,
        )

        async def step_changed() -> None:
            nonlocal changed_env, changed, changed_requests
            nonlocal sdk_accept_error_code, json_accept_exit
            changed_env = await anyio.to_thread.run_sync(
                lambda: TestEnv.open(
                    "J-032-sdk-python-changed-key",
                    os.environ,
                    proving_ground_seed=CHANGED_SEED,
                )
            )
            changed_server = changed_env.proving_ground
            assert changed_server is not None, "changed-key Proving ground started"
            changed = await _run_crawl(client, changed_server, "changed-key")
            observations = await anyio.to_thread.run_sync(changed_server.observations)
            requests = cast("Sequence[Mapping[str, object]]", observations["requests"])
            changed_requests = sum(
                1 for request in requests if request.get("host") == HOST
            )

            # Verify `certs.accept` is refused through the SDK (`person_required`).
            listed = cast("dict[str, object]", await client.call("certs.list", {}))
            pins = cast("list[dict[str, object]]", listed.get("pins", []))
            presented = next(
                (
                    cast("str", pin["changed_spki_sha256"])
                    for pin in pins
                    if pin.get("host") == HOST
                    and isinstance(pin.get("changed_spki_sha256"), str)
                ),
                "",
            )
            assert presented, f"expected pending changed pin for {HOST}: {listed}"

            try:
                await client.call(
                    "certs.accept",
                    {"host": HOST, "spki_sha256": presented},
                )
                sdk_accept_error_code = "answered"
            except GoodfetchError as error:
                sdk_accept_error_code = error.code

            # Verify non-interactive `--json` CLI path also refuses `certs accept`.
            def run_json_accept() -> int:
                proc = run.command(["certs", "accept", HOST, "--json"])
                proc.communicate()
                return proc.returncode

            json_accept_exit = await anyio.to_thread.run_sync(run_json_accept)

        await run.step(
            "Crawl the site the same way once its server presents another key",
            step_changed,
        )

        async def step_accept() -> None:
            exit_code, output = await anyio.to_thread.run_sync(
                _accept_cert_in_terminal,
                run.test_env.tools.path(Tool.TEST_ENV),
                run.engine,
                HOST,
                run.test_env.env,
                run.test_env.folder,
            )
            assert exit_code == 0, output

        await run.step(
            "Accept the site's new key in a terminal (`certs accept`)",
            step_accept,
        )

        async def step_again() -> None:
            nonlocal accepted
            assert changed_env is not None
            changed_server = changed_env.proving_ground
            assert changed_server is not None
            accepted = await _run_crawl(client, changed_server, "accepted")

        await run.step("Crawl the site the same way again", step_again)
    finally:
        if foxhound is not None:
            await foxhound.close()
        if changed_env is not None:
            changed_env.close()

    async def check_first() -> None:
        assert first is not None
        assert first.kind == "succeeded", first
        assert first.errors == (), first
        assert first.pinned_key is not None, first

    await run.check(
        "The first crawl fetched the site through its exception, and its "
        "Record's Provenance carries the exception and the key it pinned",
        check_first,
    )

    async def check_changed() -> None:
        assert changed is not None
        assert changed.kind == "failed", changed
        assert changed.errors == ("certificate_changed",), changed
        assert changed.record_count == 0, changed
        assert changed_requests == 0
        assert sdk_accept_error_code == "person_required"
        assert json_accept_exit != 0

    await run.check(
        "The crawl after the key changed failed the site's request with "
        "`certificate_changed`, and no request reached the site",
        check_changed,
    )

    async def check_accepted() -> None:
        assert first is not None
        assert accepted is not None
        assert accepted.kind == "succeeded", accepted
        assert accepted.errors == (), accepted
        assert accepted.pinned_key is not None, accepted
        assert accepted.pinned_key != first.pinned_key

    await run.check(
        "Once the new key was accepted, the crawl fetched the site under the new key",
        check_accepted,
    )
