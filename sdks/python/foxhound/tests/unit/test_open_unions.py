"""A Foxhound outcome from a newer engine, kept as received (ADR 0079).

An outcome whose kind this package does not know reads as the outcome union's
`unknown` member: the kind it was sent in `tag`, the fields every outcome
shares read as theirs, and the rest as sent. Collie's twin is
`tests/unit/test_open_values.py` of `goodfetch-collie-client`.
"""

from __future__ import annotations

from datetime import UTC, datetime

from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.unknown_member import UnknownMember
from goodfetch_foxhound.generated.models import CrawlRequestOutcomes


def test_an_outcome_of_a_newer_kind_reads_as_the_unknown_member() -> None:
    """Its kind in `tag`, its shared `ended_at` read, its own field kept."""
    sent: dict[str, object] = {
        "kind": "a_newer_kind",
        "attempts": 1,
        "errors": [],
        "position": 1,
        "request": {"id": "r1", "url": "https://shop.test/", "label": "page"},
        "ended_at": "2026-11-19T08:00:00Z",
        "its_own_field": "kept",
    }

    outcomes = result_of(CrawlRequestOutcomes, {"outcomes": [sent]})

    outcome = outcomes.outcomes[0].root
    assert isinstance(outcome, UnknownMember)
    assert (outcome.tag, outcome.model_dump()["ended_at"], outcome.model_extra) == (
        "a_newer_kind",
        datetime(2026, 11, 19, 8, tzinfo=UTC),
        {"its_own_field": "kept"},
    )
