"""A start reaches the Run its Run ID names (ADRs 0078 and 0102).

A fresh Run ID begins the Run, an interrupted Run's resumes it and a finished
Run's returns it ended, each start the engine refuses raising its generated
class. The case plays on the Scripted engine started as the executable, which
checks every message the SDK sends, three ways: async on asyncio, async on
Trio and through the sync `Collie`; the last test runs a start's answers on
the executable of the same commit, with the Run ID off its pattern that no
script may send. TypeScript's `start.test.ts` is its twin.
"""

import logging
from collections.abc import Mapping

import pytest
from collie_calls import (
    CLIENT_LOGGER,
    START_LOGGER,
    failure,
    open_collie,
    start_lines,
)
from goodfetch_collie import (
    LimitReachedError,
    RunConflictError,
    RunEndedError,
    RunIdInvalidError,
    RunInUseError,
    StartedAs,
)
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct


@pytest.mark.covers("story:11-126", "story:11-127", "operation:run.started_as")
@pytest.mark.client_case(id="start-reaches-its-run", mode="attached")
@three_ways
async def test_reaches_the_run_its_run_id_names_and_raises_each_refusals_class(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """New, resumed and ended each answer the start; a resume and an end are logged."""
    caplog.set_level(logging.INFO, logger=START_LOGGER)
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/start-reaches-its-run.json"
    )
    collie = await open_collie(way, env)

    begun = await collie.start(run_id="nightly", tags=["shop", "eu"])
    run_id = begun.id
    await begun.close()
    resumed = await collie.start(run_id="nightly")
    await resumed.finish()
    ended = await collie.start(run_id="nightly")
    no_work = await failure(ended.new_page())
    in_use = await failure(collie.start(run_id="busy"))
    conflict = await failure(collie.start(run_id="crawled"))
    reached = await failure(collie.start(run_id="capped"))
    await collie.close()

    with client_case.check(
        "run.started_as answers how each start reached the Run, "
        "without a message to the engine"
    ):
        assert [run.started_as for run in (begun, resumed, ended)] == [
            StartedAs.new,
            StartedAs.resumed,
            StartedAs.ended,
        ]
    with client_case.check("run.id names the Run without a message to the engine"):
        assert run_id == "nightly"
    with client_case.check(
        "the resume is logged at info, the ended Run at warning, on its logger"
    ):
        assert start_lines(caplog) == [
            (
                logging.INFO,
                "The start resumed the Run `nightly` from its committed state.",
                "nightly",
                "resumed",
            ),
            (
                logging.WARNING,
                "The start found the Run `nightly` ended: it is returned as it "
                "ended and takes no new work.",
                "nightly",
                "ended",
            ),
        ]
    with client_case.check(
        "a call that would give the ended Run work raises run_ended"
    ):
        assert isinstance(no_work, RunEndedError)
        assert (no_work.code, no_work.group, no_work.run_id, no_work.details) == (
            "run_ended",
            "program",
            "nightly",
            {},
        )
    with client_case.check("a Run another engine writes raises run_in_use"):
        assert isinstance(in_use, RunInUseError)
        assert (in_use.code, in_use.retryable, in_use.run_id, in_use.details) == (
            "run_in_use",
            True,
            "busy",
            {},
        )
    with client_case.check("a Run of other inputs raises run_conflict listing each"):
        assert isinstance(conflict, RunConflictError)
        assert (conflict.code, conflict.run_id, conflict.details) == (
            "run_conflict",
            "crawled",
            {"differences": ["product", "kind", "caller", "seeds"]},
        )
    with client_case.check("a Run whose Limit is used up raises limit_reached"):
        assert isinstance(reached, LimitReachedError)
        assert (
            reached.code,
            reached.details["scope"],
            reached.details["usage_kind"],
            reached.details["resets_at"],
        ) == (
            "limit_reached",
            {"kind": "run", "id": "capped"},
            "requests",
            "2026-02-01T00:00:00Z",
        )
    with client_case.check(
        "the script played to its end, so its engine wrote no failure"
    ):
        assert [
            record.getMessage()
            for record in caplog.records
            if record.name == CLIENT_LOGGER
        ] == []


@pytest.mark.covers("story:11-18", "story:11-126", "story:11-127")
@three_ways
async def test_begins_resumes_and_returns_a_finished_run_ended_on_the_executable(
    way: Way, env: Mapping[str, str]
) -> None:
    """The engine of the same commit refuses a live Run and tags past their bounds."""
    collie = await open_collie(way, env)

    begun = await collie.start(run_id="held", tags=["shop"])
    in_use = await failure(collie.start(run_id="held"))
    await begun.close()
    resumed = await collie.start(run_id="held")
    await resumed.finish()
    ended = await collie.start(run_id="held")
    invalid = await failure(collie.start(run_id="Held"))
    too_many = await failure(collie.start(tags=["t"] * 17))
    too_long = await failure(collie.start(tags=["t" * 65]))
    await collie.close()

    assert [run.started_as for run in (begun, resumed, ended)] == [
        StartedAs.new,
        StartedAs.resumed,
        StartedAs.ended,
    ]
    assert (in_use.code, in_use.run_id) == ("run_in_use", "held")
    assert isinstance(invalid, RunIdInvalidError)
    assert isinstance(invalid.details["pattern"], str)
    assert [error.code for error in (invalid, too_many, too_long)] == [
        "run_id_invalid",
        "invalid_params",
        "invalid_params",
    ]
