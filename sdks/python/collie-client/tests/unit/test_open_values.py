"""A newer engine's values, kept as received (ADR 0079).

A Verdict or Limit scope this package does not know reads as the string the
engine sent, typed by its open enumeration, and an unknown Verdict is never
`verified`. Foxhound's values of a type both Products share reach a Collie
program the same way (ADR 0095). The TypeScript twin is Foxhound's
`tests/open-values.test.ts`.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.generated.models import LimitScope, RunReport

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
