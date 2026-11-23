"""Unknown values from a newer engine (ADR 0079), on the Scripted engine.

The Scripted engine plays a Collie engine, three ways: async on asyncio,
async on Trio and through the sync `Collie`. Each value the Client reads
that no catalog of this package has is kept as received and raises
nothing of its own. A code no catalog has raises its group's class alone,
with its details; `call_cancelled` keeps a canceller and a closed value it
does not know, and `file_write_failed` a reason no list has; and the Run's
report keeps its Verdict, never `verified`, its Stop reason as the union's
`unknown` member, a warning code no table has, and Foxhound's Usage kind
and Limit scope, values of types both Products share. Played attached, and
as a Daemon, whose script exits 0 only once every step played.
TypeScript's `unknown-values.test.ts` is its twin.
"""

import logging
from dataclasses import dataclass

import pytest
from collie_calls import engine_warnings, exit_code, failure, locator_on, open_collie
from goodfetch_collie import (
    AsyncCollie,
    CallCancelledError,
    FileWriteFailedError,
    GoodfetchError,
    PageError,
    RunReport,
)
from goodfetch_collie_client.unknown_member import UnknownMember
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    start_scripted_daemon,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

RUN_ID = "01ke6p4ym0j8x3v5wq2rtbn7hc"
"""The Run the script starts."""

COVERS = pytest.mark.covers("story:11-33", "story:11-188")
"""What both modes' tests cover."""


@dataclass(frozen=True, slots=True)
class Played:
    """What the case's calls answered, in the script's order."""

    unknown_code: GoodfetchError
    cancelled: GoodfetchError
    write_failed: GoodfetchError
    report: RunReport


async def played(way: Way, collie: AsyncCollie) -> Played:
    """Plays the case's calls on `collie`, then closes it."""
    run = await collie.start()
    page = await run.new_page()
    unknown_code = await failure(page.goto("https://shop.test/"))
    cancelled = await failure(locator_on(way, page, "#next").click())
    write_failed = await failure(locator_on(way, page, "#save").click())
    report = await run.finish()
    await collie.close()
    return Played(
        unknown_code=unknown_code,
        cancelled=cancelled,
        write_failed=write_failed,
        report=report,
    )


def check_played(client_case: ClientCaseRecord, answered: Played) -> None:
    """Each value no catalog of this package has was kept as the engine sent it."""
    unknown_code = answered.unknown_code
    cancelled = answered.cancelled
    write_failed = answered.write_failed
    report = answered.report
    with client_case.check(
        "a code no catalog has raises its group's class alone, its code and"
        " details kept"
    ):
        assert type(unknown_code) is PageError
        assert (
            unknown_code.code,
            unknown_code.group,
            unknown_code.retryable,
            unknown_code.operation,
            unknown_code.call_id,
            unknown_code.run_id,
            unknown_code.url,
        ) == (
            "frame_frozen",
            "page",
            False,
            "page.goto",
            "call-1",
            RUN_ID,
            "https://shop.test/",
        )
        assert unknown_code.details == {"frozen_for": 1200}
    with client_case.check(
        "call_cancelled keeps a canceller and a closed value no catalog has, as sent"
    ):
        assert isinstance(cancelled, CallCancelledError)
        assert (cancelled.code, cancelled.target) == ("call_cancelled", "#next")
        assert cancelled.typed_details is not None
        assert (cancelled.typed_details.by, cancelled.typed_details.closed) == (
            "satellite",
            "tab_group",
        )
    with client_case.check("file_write_failed keeps a reason no list has, as sent"):
        assert isinstance(write_failed, FileWriteFailedError)
        assert (write_failed.code, write_failed.target) == (
            "file_write_failed",
            "#save",
        )
        assert write_failed.details == {
            "reason": "crosses_devices",
            "os_code": "EXDEV",
            "path": f"/srv/goodfetch/runs/{RUN_ID}",
            "side": "engine",
        }
    with client_case.check(
        "an unknown Verdict is kept as sent and never reads as verified"
    ):
        assert report.verdict == "verified_by_quorum"
        assert report.verdict != "verified"
    with client_case.check(
        "an unknown Stop reason reads as the union's unknown member, its tag and"
        " fields kept"
    ):
        stop = report.stop_reason.root if report.stop_reason else None
        assert isinstance(stop, UnknownMember)
        assert stop.model_dump() == {
            "kind": "unknown",
            "tag": "budget_window",
            "window": "night",
        }
    with client_case.check(
        "Foxhound's Usage kind and Limit scope, and a warning code no table has,"
        " are kept as strings"
    ):
        assert [
            (other.usage_kind, other.amount, other.currency)
            for other in report.usage.other or []
        ] == [("paid_fetch", 3, None)]
        assert [
            (limit.scope.kind, limit.scope.id, limit.usage_kind)
            for limit in report.usage.limits or []
        ] == [("partitioned_crawl", "books", "paid_fetch")]
        assert [warning.code for warning in report.warnings or []] == ["quorum_slow"]


@COVERS
@pytest.mark.client_case(id="unknown-values-are-kept", mode="attached")
@three_ways
async def test_keeps_each_value_a_newer_engine_sends_as_it_was_sent(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Each unknown value kept as sent, raising nothing of its own."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/unknown-values-are-kept.json"
    )
    caplog.set_level(logging.DEBUG, logger="goodfetch")
    answered = await played(way, await open_collie(way, env))

    check_played(client_case, answered)
    with client_case.check(
        "the script played to its end, so its engine wrote no failure"
    ):
        assert engine_warnings(caplog) == []


@COVERS
@pytest.mark.client_case(id="unknown-values-are-kept", mode="daemon")
@three_ways
async def test_keeps_the_same_values_from_a_daemon(
    way: Way,
    test_env: TestEnv,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """The same unknown values from a Daemon, each kept as sent."""
    daemon = start_scripted_daemon(
        test_env, "sdk-python/run/unknown-values-are-kept-from-a-daemon.json", "socket"
    )
    caplog.set_level(logging.DEBUG, logger="goodfetch")
    collie = await open_collie(way, test_env.env, engine="daemon")
    answered = await played(way, collie)
    code = await exit_code(daemon.wait)

    check_played(client_case, answered)
    with client_case.check(
        "the Daemon played its script through, so the Client sent exactly what it holds"
    ):
        assert code == 0
