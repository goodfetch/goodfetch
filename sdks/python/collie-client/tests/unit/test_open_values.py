"""A newer engine's values, kept as received (ADR 0079).

A Verdict or Limit scope this package does not know reads as the string the
engine sent, typed by its open enumeration, a member of an open tagged union
it does not know reads as its `unknown` member with the tag it received, and
an unknown Verdict is never `verified`. Foxhound's values of a type both
Products share reach a Collie program the same way (ADR 0095). The TypeScript
twin is Foxhound's `tests/open-values.test.ts`.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.generated.models import LimitScope, RunReport
from goodfetch_collie_client.unknown_member import UnknownMember

if TYPE_CHECKING:
    from goodfetch_collie_client.generated.open_values import Verdict
    from goodfetch_collie_client.generated.shared_types import UsageKind


def test_an_unknown_verdict_is_kept_and_is_not_verified() -> None:
    """A Verdict a newer engine adds is the string it sent, never `verified`."""
    report = result_of(RunReport, {"run_id": "run-1", "verdict": "a_newer_verdict"})

    assert (report.verdict, report.verdict == "verified") == ("a_newer_verdict", False)


def test_a_value_of_foxhounds_own_reaches_a_collie_program_as_a_string() -> None:
    """Foxhound's Usage kind and Limit scope kind are kept by Collie's reader."""
    scope = result_of(LimitScope, {"kind": "partitioned_crawl", "id": "books"})
    usage: UsageKind = "paid_fetches"

    assert (scope.kind, usage) == ("partitioned_crawl", "paid_fetches")


def test_each_open_enumeration_accepts_any_other_string() -> None:
    """An open enumeration's alias types any other string too."""
    verdict: Verdict = "a_newer_verdict"

    assert verdict == "a_newer_verdict"


def test_a_member_of_an_open_tagged_union_a_newer_engine_adds_keeps_its_tag() -> None:
    """An unknown Stop reason is the `unknown` member, its tag and the rest kept."""
    report = result_of(
        RunReport,
        {"run_id": "run-1", "stop_reason": {"kind": "a_newer_reason", "by_whom": "x"}},
    )

    stop = report.stop_reason.root if report.stop_reason else None
    assert isinstance(stop, UnknownMember)
    assert (stop.tag, stop.model_dump()) == (
        "a_newer_reason",
        {"kind": "unknown", "tag": "a_newer_reason", "by_whom": "x"},
    )


def test_a_member_of_an_open_tagged_union_this_package_knows_reads_as_its_own() -> None:
    """A Stop reason this package knows is never the `unknown` member."""
    report = result_of(RunReport, {"run_id": "run-1", "stop_reason": {"kind": "pause"}})

    stop = report.stop_reason.root if report.stop_reason else None
    assert stop is not None
    assert (stop.kind, isinstance(stop, UnknownMember)) == ("pause", False)
