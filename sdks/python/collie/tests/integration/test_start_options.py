"""A start's options (ADRs 0033, 0080, 0091, 0102 and 0105).

The `start-takes-every-option` client case. On the Scripted engine, which
checks every message the SDK sends, a start sends its settings, its
`timeout`, its Plan file, its Response recording and each Plugin change it
accepts, and `timeout=None` sends null; a zero
`timeout` is refused before anything is sent; and a start whose task is
cancelled once it is sent is given up with one `$/cancel` naming it, async on
asyncio and on Trio. On the Daemon of Collie's executable of the same commit,
three ways, a start's own `timeout` stops the Run with a Checkpoint, a start
of its Run ID resumes it, and another Plan file or Response recording is
`run_conflict` naming each. TypeScript's `start-options.test.ts` and the
Rust client crate's `start_options.rs` are its twins.
"""

from collections.abc import Mapping
from datetime import timedelta

import anyio
import pytest
from collie_calls import engine_warnings, failure, open_collie
from goodfetch_collie import (
    GoodfetchError,
    Recording,
    RecordingMode,
    RunConflictError,
    RunOrigin,
    SettingInvalidError,
    SettingsLayer,
)
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_client_methods,
    scripted_engine_env,
    three_ways,
    until_scripted_engine_read,
)
from goodfetch_test_support import Product as ToolProduct

SCRIPT = "sdk-python/run/start-takes-every-option.json"
"""The script the Attached engine plays."""

ZERO_TIMEOUT = ("timeout.run", 0, SettingsLayer.run, RunOrigin.argument)
"""The limit a zero `timeout` names: the Run's timeout, from the argument."""


def assert_zero_timeout_refused(zero: GoodfetchError) -> None:
    """Checks `zero` is `setting_invalid`, its limit and fix the null form's."""
    assert isinstance(zero, SettingInvalidError)
    assert zero.code == "setting_invalid"
    assert zero.limit is not None
    assert (
        zero.limit.key,
        zero.limit.value,
        zero.limit.layer,
        zero.limit.origin,
    ) == ZERO_TIMEOUT
    assert zero.fix is not None
    assert "`timeout=None`" in zero.fix


@pytest.mark.covers("story:11-50", "story:11-132", "operation:run.start")
@pytest.mark.client_case(id="start-takes-every-option", mode="attached")
@pytest.mark.parametrize(
    ("anyio_backend", "way"),
    [
        pytest.param("asyncio", Way.ASYNCIO, id="asyncio"),
        pytest.param("trio", Way.TRIO, id="trio"),
    ],
)
@pytest.mark.anyio
async def test_a_start_sends_every_option_and_a_cancelled_one_sends_cancel(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Every option and a null timeout are sent; a zero timeout is not."""
    env = scripted_engine_env(test_env, product, SCRIPT)
    collie = await open_collie(way, env)
    resumed = await collie.start(
        run_id="nightly",
        settings={"timeout": {"wait": timedelta(seconds=5)}},
        timeout=timedelta(hours=1),
        plan="plans/nightly.json",
        recording=Recording(mode=RecordingMode.replay, path="recordings/nightly.warc"),
        accept_plugin_change=["shop-prices"],
    )
    await resumed.close()
    ended = await collie.start(run_id="done", timeout=None)
    await ended.close()
    zero = await failure(collie.start(run_id="never", timeout=timedelta(0)))

    propagated = False

    async def slow_start() -> None:
        nonlocal propagated
        try:
            await collie.start(run_id="slow")
        except anyio.get_cancelled_exc_class():
            propagated = True
            raise

    async with anyio.create_task_group() as starts:
        starts.start_soon(slow_start)
        await until_scripted_engine_read(test_env.home, "run.start", 3)
        starts.cancel_scope.cancel()
    await collie.close()

    with client_case.check("each start answers its Run as the engine started it"):
        assert [resumed.started_as, ended.started_as] == ["resumed", "ended"]
    with client_case.check(
        "a zero timeout is setting_invalid from the argument, naming the null form"
    ):
        assert_zero_timeout_refused(zero)
    with client_case.check(
        "a start whose task is cancelled propagates the cancellation after one $/cancel"
    ):
        assert propagated
        assert scripted_engine_client_methods(test_env.home) == [
            "hello",
            "run.start",
            "run.abort",
            "run.start",
            "run.start",
            "$/cancel",
        ]
    with client_case.check("the engine's call_cancelled answer warns of nothing"):
        assert engine_warnings(caplog) == []


@pytest.mark.covers("story:11-50", "story:11-132")
@pytest.mark.client_case(id="start-takes-every-option", mode="daemon")
@pytest.mark.parametrize("with_daemon", [True])
@three_ways
async def test_a_start_timeout_stops_the_run_and_other_inputs_conflict(
    way: Way, env: Mapping[str, str], client_case: ClientCaseRecord
) -> None:
    """The Daemon of the same commit stops, resumes and refuses as the options say."""
    collie = await open_collie(way, env, engine="daemon")
    timed = await collie.start(run_id="timed", timeout=timedelta(milliseconds=1))
    timed_out = await timed.report()
    resumed = await collie.start(run_id="timed", timeout=timedelta(hours=1))
    await resumed.finish()
    zero = await failure(collie.start(timeout=timedelta(0)))

    first = await collie.start(
        run_id="planned",
        plan="plans/a.json",
        recording=Recording(mode=RecordingMode.record, path="recordings/a.warc"),
        accept_plugin_change=["shop-prices"],
        timeout=None,
    )
    await first.close()
    conflict = await failure(collie.start(run_id="planned", plan="plans/b.json"))
    await collie.close()

    with client_case.check("a start's own timeout stops the Run with a Checkpoint"):
        assert (timed_out.lifecycle, timed_out.completeness) == (
            "stopped",
            "incomplete",
        )
        assert timed_out.stop_reason is not None
        assert timed_out.stop_reason.root.kind == "timeout"
        assert timed_out.checkpoint is not None
    with client_case.check("a start of its Run ID resumes it"):
        assert resumed.started_as == "resumed"
    with client_case.check(
        "a zero timeout is setting_invalid from the argument, naming the null form"
    ):
        assert_zero_timeout_refused(zero)
    with client_case.check(
        "another Plan file and no recording are run_conflict naming each"
    ):
        assert isinstance(conflict, RunConflictError)
        assert (conflict.code, conflict.details) == (
            "run_conflict",
            {"differences": ["plan", "recording"]},
        )
