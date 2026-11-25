"""A Connection's Schedules (spec 589, ADR 0099) on an Attached engine.

The `schedules-crud` client conformance case, on the executable of the
same commit, three ways: async on asyncio, async on Trio and through the
sync `Collie`. `schedules.create` answers a Schedule under the ID given, or
one it made when none was, `schedules.list` lists both, `schedules.get`
answers the handle of a held ID and raises `ScheduleNotFoundError` for an
unknown one, and a handle's `info()` reads what the store holds and its
`delete()` answers whether it deleted it. TypeScript's `schedules.test.ts`
is its twin.
"""

import re
from collections.abc import Mapping
from datetime import timedelta

import pytest
from collie_calls import failure, open_collie
from goodfetch_collie import ScheduleDefinition, ScheduleNotFoundError
from goodfetch_test_support import ClientCaseRecord, Way, three_ways


@pytest.mark.covers("story:10-2")
@pytest.mark.client_case(id="schedules-crud", mode="attached")
@three_ways
async def test_creates_lists_reads_and_deletes_a_schedule(
    way: Way, env: Mapping[str, str], client_case: ClientCaseRecord
) -> None:
    """Each Schedule call reaches the store; an unknown ID raises its class."""
    collie = await open_collie(way, env)
    schedules = way.api(collie.schedules, collie.schedules)

    named = await schedules.create(
        ScheduleDefinition.model_validate(
            {
                "id": "nightly-prices",
                "target": {"recipe": "shop-prices"},
                "trigger": {"every": 3_600_000},
                "time_zone": "Europe/Paris",
            }
        )
    )
    made = await schedules.create(
        ScheduleDefinition.model_validate(
            {"target": {"recipe": "shop-prices"}, "trigger": {"every": 60_000}}
        ),
        timeout=timedelta(seconds=10),
    )
    listed = await schedules.list()
    held = await schedules.get("nightly-prices", timeout=None)
    got = way.api(held, held)
    info = await got.info()
    unknown = await failure(schedules.get("never-created"))
    deleted = await way.api(named.schedule, named.schedule).delete()
    again = await got.delete()
    gone = await failure(got.info())
    await collie.close()

    with client_case.check(
        "create answers the Schedule under the ID given, with no warnings"
    ):
        assert (named.schedule.id, named.warnings) == ("nightly-prices", ())
    with client_case.check("create makes an ID when the definition names none"):
        assert re.fullmatch(r"s[0-9a-z]{26}", made.schedule.id)
    with client_case.check("list holds both Schedules"):
        assert sorted(each.schedule_id.root for each in listed) == sorted(
            [made.schedule.id, "nightly-prices"]
        )
    with client_case.check(
        "get answers the handle, whose info reads the stored Schedule"
    ):
        assert got.id == "nightly-prices"
        assert info.model_dump(
            mode="json",
            include={
                "schedule_id",
                "target",
                "trigger",
                "time_zone",
                "version",
                "state",
            },
            exclude_none=True,
        ) == {
            "schedule_id": "nightly-prices",
            "target": {"recipe": "shop-prices"},
            "trigger": {"every": 3_600_000},
            "time_zone": "Europe/Paris",
            "version": 1,
            "state": "on",
        }
    with client_case.check("get of an unknown ID raises ScheduleNotFoundError"):
        assert isinstance(unknown, ScheduleNotFoundError)
        assert unknown.typed_details is not None
        assert unknown.typed_details.schedule_id.root == "never-created"
    with client_case.check("delete answers whether it deleted, and info then fails"):
        assert (deleted, again) == (True, False)
        assert isinstance(gone, ScheduleNotFoundError)
