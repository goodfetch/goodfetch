"""Every `date-time` is a time-zone-aware `datetime` in UTC (ADR 0092).

The engine may write a point in time with any offset: a result or an event
holds the same instant in UTC, and a `datetime` a program sends crosses the
wire as RFC 3339 in UTC (spec 606, Casing and times).
"""

from datetime import UTC, datetime, timedelta, timezone

import pytest
from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.generated.events import EVENTS
from goodfetch_collie_client.generated.models import (
    RunId,
    RunListParams,
    RunProgress,
    RunProgressParams,
    TimeRange,
)
from goodfetch_collie_client.generated.operations import CollieOperations
from goodfetch_test_support import EngineAnswering

pytestmark = pytest.mark.covers("story:11-91")

PAUSE = {
    "page": "page-1",
    "raised_by": {"by": "program"},
    "session": "session-1",
    "since": "2026-11-19T04:00:00-05:00",
    "deadline": "2026-11-20T09:00:00+05:30",
}

PROGRESS = {
    "at": "2026-11-19T10:30:00.123456+02:00",
    "checkpoint_at": None,
    "epoch": 1,
    "last": False,
    "lifecycle": "running",
    "pauses": [PAUSE],
    "position": 3,
    "run_id": "run-1",
    "writer": "live",
}


def is_utc(point: object, expected: datetime) -> bool:
    """Whether `point` is `expected` as a `datetime` whose zone is UTC itself."""
    return isinstance(point, datetime) and point.tzinfo is UTC and point == expected


def test_a_results_times_are_utc_whatever_offset_the_engine_wrote() -> None:
    """A time with an offset, nested or nullable ones too, reads in UTC."""
    operations = CollieOperations(EngineAnswering(PROGRESS))

    progress = operations.run_progress(RunProgressParams(run_id=RunId(root="run-1")))

    assert is_utc(progress.at, datetime(2026, 11, 19, 8, 30, 0, 123456, tzinfo=UTC))
    assert is_utc(progress.pauses[0].since, datetime(2026, 11, 19, 9, 0, tzinfo=UTC))
    assert is_utc(
        progress.pauses[0].deadline, datetime(2026, 11, 20, 3, 30, tzinfo=UTC)
    )


def test_a_null_time_stays_none() -> None:
    """A nullable `date-time` the engine sends as null is None."""
    operations = CollieOperations(EngineAnswering(PROGRESS))

    progress = operations.run_progress(RunProgressParams(run_id=RunId(root="run-1")))

    assert progress.checkpoint_at is None


def test_an_events_times_are_utc() -> None:
    """An event's payload, built as its registered model, holds its times in UTC."""
    event = result_of(EVENTS["run.progress"], {**PROGRESS, "at": "2026-11-19T08:30Z"})

    assert isinstance(event, RunProgress)
    assert is_utc(event.at, datetime(2026, 11, 19, 8, 30, tzinfo=UTC))


def test_a_time_no_point_in_time_reads_is_kept_as_sent() -> None:
    """A value no point in time reads neither raises nor is dropped (ADR 0082)."""
    operations = CollieOperations(
        EngineAnswering({**PROGRESS, "at": "2026-11-19T10:30:00"})
    )

    progress = operations.run_progress(RunProgressParams(run_id=RunId(root="run-1")))

    sent: object = progress.at
    assert sent == "2026-11-19T10:30:00"


def test_a_sent_datetime_crosses_the_wire_as_rfc3339_in_utc() -> None:
    """A `datetime` with any offset is sent as the same instant, written in UTC."""
    engine = EngineAnswering({"runs": []})
    paris = timezone(timedelta(hours=2))

    CollieOperations(engine).run_list(
        RunListParams(
            started=TimeRange.model_validate(
                {"from": datetime(2026, 11, 19, 10, tzinfo=paris)}
            )
        )
    )

    [params] = engine.sent
    wire = params.model_dump(mode="json", by_alias=True, exclude_unset=True)
    assert wire == {"started": {"from": "2026-11-19T08:00:00Z"}}
