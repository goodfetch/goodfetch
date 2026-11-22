"""Reaching a Daemon again after its link dropped (spec 606).

The backoff between attempts, 250 ms, doubling, capped at 10 s, all within
`timeout.connect`, and what `events.subscribe` is sent again with: after
the last event it read, with the credits it still had. TypeScript's
`reconnect.test.ts` and `resumed-stream.test.ts` are its twins.
"""

import pytest
from goodfetch_collie_client._relink import reconnect_delay
from goodfetch_collie_client.listeners import EVENTS, ResumedEvents


def _batch(*positions: int) -> dict[str, object]:
    return {
        "events": [
            {"event": "session.page", "data": {}, "position": position}
            for position in positions
        ]
    }


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


@pytest.mark.covers("story:11-6")
def test_resubscribes_from_the_start_before_any_event_was_read() -> None:
    """No position yet: `{}`, with every credit it granted."""
    resumed = ResumedEvents(None, lambda: "0123456789abcdef")
    resumed.granted(16)

    assert resumed.resumed() == ({}, 16)


@pytest.mark.covers("story:11-6")
def test_resubscribes_after_the_last_event_read_with_the_credits_it_still_had() -> None:
    """Granted less received, after the last position of the engine instance."""
    heard: list[object] = []
    resumed = ResumedEvents(lambda _, params: heard.append(params), lambda: "e1")
    resumed.granted(16)
    resumed.notified(EVENTS, _batch(1, 2))
    resumed.granted(2)
    resumed.notified(EVENTS, _batch(3))

    assert resumed.resumed() == (
        {"after": {"engine_instance": "e1", "position": 3}},
        15,
    )
    assert heard == [_batch(1, 2), _batch(3)]
