"""The backoff between attempts to reach a Daemon again after its link dropped.

250 ms, doubling, capped at 10 s, all within `timeout.connect` (spec 606).
TypeScript's `reconnect.test.ts` is its twin.
"""

import pytest
from goodfetch_collie_client._relink import reconnect_delay


@pytest.mark.covers("story:11-6")
def test_waits_250_ms_before_the_first_attempt_and_doubles_each_time_up_to_10_s() -> (
    None
):
    """Each attempt waits twice the last, never more than 10 s."""
    delays = [reconnect_delay(attempt, 0, None) for attempt in range(9)]

    assert delays == [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 10.0, 10.0, 10.0]


@pytest.mark.covers("story:11-6")
def test_gives_up_once_the_next_attempt_would_start_past_timeout_connect() -> None:
    """Counted from the drop: an attempt starting at the bound is not made."""
    assert {
        "inside": reconnect_delay(3, 27.999, 30.0),
        "at_the_end": reconnect_delay(3, 28.0, 30.0),
        "past": reconnect_delay(6, 25.75, 30.0),
    } == {"inside": 2.0, "at_the_end": None, "past": None}
