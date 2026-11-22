"""How a Run ends and what it does meanwhile (ADRs 0029 and 0102).

`finish()` and `stop()` answer the Run report, a failed Run raises its
error and a call's timeout ends only its wait, while `progress()` follows
the Run, on an Attached engine that plays a Scripted engine's script
(ADR 0069), three ways: async on asyncio, async on Trio and through the
sync `Collie`. The script fails on a message it does not expect and on a
step left unused, and the engine writes that failure to its stderr, which
reaches the `goodfetch.client` logger at warning. The guest program of
`nothing-at-import-or-connect` follows the progress in a process of its own
and finds nothing on stdout or stderr: no status line. TypeScript's
`ending-a-run.test.ts` is its twin.
"""

from datetime import UTC, datetime, timedelta
from typing import cast

import anyio
import anyio.from_thread
import pytest
from collie_calls import engine_warnings, failure, open_collie
from goodfetch_collie import (
    AsyncRun,
    BrowserLaunchFailedError,
    DeadlineExceededError,
    Run,
    RunProgress,
)
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

MESSAGE = "three pages left"


def shown(value: RunProgress) -> tuple[int, str | None]:
    """A progress value's position and the program's message it carries."""
    return value.position, None if value.message is None else value.message.root


async def follow_while_setting(
    way: Way, run: AsyncRun
) -> tuple[list[RunProgress], RunProgress]:
    """`run`'s first progress value, then the one after the message it sets.

    Read this way's, and the progress set between them.
    """
    if way is Way.SYNC:
        sync = cast("Run", run)

        def follow() -> tuple[list[RunProgress], RunProgress]:
            with sync.progress() as progress:
                first = next(progress)
                # The way's Run hands each call to a worker thread of its own.
                set_ = anyio.from_thread.run(run.set_progress_message, MESSAGE)
                return [first, next(progress)], set_

        return await anyio.to_thread.run_sync(follow)
    async with run.progress() as progress:
        first = await anext(progress)
        set_ = await run.set_progress_message(MESSAGE)
        return [first, await anext(progress)], set_


@pytest.mark.client_case(id="finish-stop-and-progress", mode="attached")
@pytest.mark.covers("story:11-128", "story:11-129", "error:deadline_exceeded")
@three_ways
async def test_ends_with_the_report_raises_a_failed_runs_error_and_follows_progress(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Finish and stop answer the report; a timeout ends only the wait."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/finish-stop-and-progress.json"
    )
    collie = await open_collie(way, env)
    nightly = await collie.start(run_id="nightly")
    progress, set_ = await follow_while_setting(way, nightly)
    waited = await failure(nightly.finish(timeout=timedelta(seconds=60)))
    finished = await nightly.finish()
    weekly = await collie.start(run_id="weekly")
    stopped = await weekly.stop()
    broken = await collie.start(run_id="broken")
    failed = [await failure(broken.finish()), await failure(broken.stop())]
    await collie.close()

    with client_case.check(
        "the progress answers at once, then the value after it carries the"
        " program's message"
    ):
        assert [shown(value) for value in progress] == [(1, None), (2, MESSAGE)]
    with client_case.check("set_progress_message answers the progress it set"):
        assert shown(set_) == (2, MESSAGE)
    with client_case.check(
        "a finish's timeout fails it with deadline_exceeded, ending only the wait"
    ):
        assert isinstance(waited, DeadlineExceededError)
        assert finished.lifecycle == "finished"
    with client_case.check("finish answers the whole Run report"):
        assert (
            finished.run_id.root,
            finished.verdict,
            finished.completeness,
            finished.usage.requests,
            finished.started_at,
            finished.ended_at,
        ) == (
            "nightly",
            "needs_review",
            "complete",
            3,
            datetime(2026, 1, 1, tzinfo=UTC),
            datetime(2026, 1, 1, 0, 1, tzinfo=UTC),
        )
        assert [
            (reason.code, reason.count) for reason in finished.verdict_reasons or []
        ] == [("uncertain_effect", 1)]
        assert [
            (setting.key, setting.value, setting.layer)
            for setting in finished.weakened or []
        ] == [("consent.handling", "off", "run")]
        assert finished.plan is not None
        assert finished.plan.status == "unchanged"
        assert [
            (setting.key, setting.value, setting.layer)
            for setting in finished.settings or []
        ] == [("timeout.close", 30000, "default")]
        assert [
            (plugin.name, plugin.network_level) for plugin in finished.plugins or []
        ] == [("shop-login", "none")]
        assert [warning.code for warning in finished.warnings or []] == ["storage_low"]
        assert finished.recording is not None
        assert (
            finished.recording.mode,
            finished.recording.file,
            finished.recording.responses_used,
        ) == ("record", "nightly.recording", 0)
        assert finished.nothing_done is not None
        assert finished.nothing_done.root.reason == "nothing_pending"
        assert [
            (effect.entry, effect.since) for effect in finished.uncertain_effects or []
        ] == [("submit-order", datetime(2026, 1, 1, 0, 0, 30, tzinfo=UTC))]
    with client_case.check(
        "stop answers the Run stopped by its program, with its Checkpoint"
    ):
        assert (stopped.lifecycle, stopped.completeness) == ("stopped", "incomplete")
        assert stopped.stop_reason is not None
        reason = stopped.stop_reason.root
        # Only an abort's variant of the reason names who stopped the Run.
        assert (reason.kind, getattr(reason, "by", None)) == ("abort", "program")
        checkpoint = stopped.checkpoint
        assert checkpoint is not None
        assert (
            checkpoint.run_id.root,
            checkpoint.journal_position,
            checkpoint.time,
        ) == ("weekly", 7, datetime(2026, 1, 1, 0, 0, 5, tzinfo=UTC))
    with client_case.check(
        "a failed Run's finish and stop raise the error that failed it"
    ):
        assert [type(error) for error in failed] == [BrowserLaunchFailedError] * 2
    with client_case.check("the SDK sent exactly what the script holds"):
        assert engine_warnings(caplog) == []
