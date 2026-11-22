"""`response_unavailable`'s `retry_after`, a point in time (ADRs 0092 and 0118).

The time the site's `Retry-After` asked for, read in UTC as Collie's
`BlockedError` reads its own; TypeScript's `errors.test.ts` of
`@goodfetch/foxhound` is its twin.
"""

from __future__ import annotations

from datetime import UTC, datetime

from goodfetch_foxhound.generated.errors import ResponseUnavailableError


def test_response_unavailable_reads_its_retry_after_as_a_point_in_time() -> None:
    """A time sent in any offset reads in UTC; a request held unsent has none."""
    asked = ResponseUnavailableError(
        "unavailable",
        details={
            "url": "https://a.test/",
            "status": 503,
            "retry_after": "2026-11-21T10:00:00+02:00",
        },
    )
    held = ResponseUnavailableError(
        "unavailable", details={"url": "https://a.test/", "status": None}
    )

    assert (asked.retry_after, held.retry_after) == (
        datetime(2026, 11, 21, 8, tzinfo=UTC),
        None,
    )
