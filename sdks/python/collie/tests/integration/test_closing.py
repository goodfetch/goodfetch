"""Closing the tree a Collie program holds (ADR 0078): Connection, Run, Session, page.

Each value closes its children first, on an Attached engine that plays a
Scripted engine's script (ADR 0069), three ways: async on asyncio, async on
Trio and through the sync `Collie`. The script fails on a message it does not
expect, `hello` included, and on a step left unused, and the engine writes
that failure to its stderr, which reaches the `goodfetch.client` logger at
warning: no such line says the SDK sent exactly what the script holds, and
that the close ended the engine's input. TypeScript's `closing.test.ts` is
its twin.
"""

import json
from collections.abc import Awaitable, Mapping
from pathlib import Path

import anyio
import pytest
from goodfetch_collie import (
    AsyncCollie,
    CallCancelledError,
    ClosedError,
    Collie,
    GoodfetchError,
    RunStateUnavailableError,
)
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    three_ways,
)
from goodfetch_test_support import Product as ToolProduct

SHOP_URL = "https://shop.test/"

ENGINE_LOGGER = "goodfetch.client"
"""The logger the engine's stderr lines go to."""

SENT_DEADLINE_SECONDS = 5.0
"""How long a test waits for the Scripted engine to record a Client's call."""


async def open_collie(way: Way, env: Mapping[str, str]) -> AsyncCollie:
    """A Connection this way opened on the Attached engine `env` names."""
    return await way.open(
        lambda: AsyncCollie.connect(env=env), lambda: Collie.connect(env=env)
    )


def engine_warnings(caplog: pytest.LogCaptureFixture) -> list[str]:
    """Each line the engines wrote on stderr, which a failed play writes."""
    return [
        record.getMessage() for record in caplog.records if record.name == ENGINE_LOGGER
    ]


async def failure(call: Awaitable[object]) -> GoodfetchError:
    """The Goodfetch error `call` raises."""
    with pytest.raises(GoodfetchError) as raised:
        await call
    return raised.value


async def until_recorded(home: Path, method: str) -> None:
    """Waits until the Scripted engine in `home` has read a Client's `method`.

    The engine records each message as it reads it, one flushed JSON line
    each, and signals nothing, so the wait polls its record.
    """
    record = home / "scripted-engine" / "record.jsonl"
    with anyio.fail_after(SENT_DEADLINE_SECONDS):
        while True:
            if record.exists():
                for line in record.read_text("utf-8").splitlines():
                    entry = json.loads(line)
                    if (
                        entry.get("from") == "client"
                        and entry.get("message", {}).get("method") == method
                    ):
                        return
            await anyio.sleep(0.01)


@pytest.mark.client_case(id="children-close-with-their-parent", mode="attached")
@three_ways
async def test_closes_each_values_children_with_it_and_refuses_every_later_call(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """A Session's close closes its page; the Connection's closes the Run's tree."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/closed-with-its-children.json"
    )
    collie = await open_collie(way, env)
    run = await collie.start()
    session = await run.new_session()
    session_page = await session.new_page()

    await session.close()
    await session_page.close()
    after_session = await failure(session_page.goto(SHOP_URL))
    run_page = await run.new_page()
    await collie.close()
    await collie.close()
    after_connection = [
        await failure(run_page.goto(SHOP_URL)),
        await failure(session.new_page()),
        await failure(run.finish()),
    ]

    with client_case.check("a page closed with its Session refuses goto with closed"):
        assert isinstance(after_session, ClosedError)
        assert (after_session.code, after_session.group) == ("closed", "program")
    with client_case.check(
        "the Run's page, its Session and the Run refuse every call with closed"
        " once the Connection is"
    ):
        assert [
            (type(error), error.code, error.group) for error in after_connection
        ] == [(ClosedError, "closed", "program")] * 3
    with client_case.check(
        "the page's close after its Session's sent nothing, and the Connection's"
        " close aborted the Run once"
    ):
        assert engine_warnings(caplog) == []


@pytest.mark.client_case(id="close-stops-a-running-run", mode="attached")
@three_ways
async def test_stops_a_running_run_it_closes_and_frees_an_ended_runs_value(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """A start of a stopped Run's ID resumes it; an ended Run's close sends nothing."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/closed-then-resumed.json"
    )
    collie = await open_collie(way, env)
    first = await collie.start(run_id="nightly")

    await first.close()
    resumed = await collie.start(run_id="nightly")
    await resumed.finish()
    await resumed.close()
    await collie.close()

    with client_case.check(
        "the close of the running Run aborted it once, the start after it named"
        " its Run ID, and the close of the finished one sent nothing"
    ):
        assert engine_warnings(caplog) == []


@pytest.mark.client_case(id="connection-close-stops-its-engine", mode="attached")
@three_ways
async def test_aborts_its_run_cancels_a_call_in_flight_and_waits_for_its_engine(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """The Connection's close: the Run's abort, the `$/cancel`, the engine's end."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/closed-during-a-finish.json"
    )
    collie = await open_collie(way, env)
    run = await collie.start(run_id="nightly")
    finished: list[GoodfetchError] = []

    async def finish() -> None:
        finished.append(await failure(run.finish()))

    async with anyio.create_task_group() as calls:
        calls.start_soon(finish)
        await until_recorded(test_env.home, "run.finish")
        await collie.close()
        warned_at_close = engine_warnings(caplog)

    with client_case.check("the finish in flight fails as cancelled by its Client"):
        [cancelled] = finished
        assert isinstance(cancelled, CallCancelledError)
        assert (cancelled.code, cancelled.group, cancelled.details) == (
            "call_cancelled",
            "unavailable",
            {"by": "client"},
        )
    with client_case.check(
        "the close aborted the Run, cancelled the finish, then waited for its"
        " engine to end: the one failure, the step left for after the close,"
        " reached the logger before close() returned"
    ):
        assert warned_at_close == [
            "goodfetch-scripted-engine: unused_step: step 8 was left unused"
        ]


@pytest.mark.client_case(id="failed-close-raises-a-typed-error", mode="attached")
@three_ways
async def test_fails_its_close_with_the_engines_typed_error_chained_to_the_unwound_one(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """A refused abort fails `close()`, and the scoped form's exit, chained."""
    closed_env = scripted_engine_env(
        test_env,
        product,
        "sdk-python/run/abort-refused.json",
        test_env.folder / "closed",
    )
    scoped_env = scripted_engine_env(
        test_env,
        product,
        "sdk-python/run/abort-refused.json",
        test_env.folder / "scoped",
    )
    collie = await open_collie(way, closed_env)
    await collie.start(run_id="nightly")
    closed = await failure(collie.close())

    thrown = RuntimeError("the program failed")

    async def scoped_async() -> None:
        async with AsyncCollie.connect(env=scoped_env) as scoped:
            await scoped.start(run_id="nightly")
            raise thrown

    def scoped_sync() -> None:
        with Collie.connect(env=scoped_env) as scoped:
            scoped.start(run_id="nightly")
            raise thrown

    with pytest.raises(GoodfetchError) as raised:
        if way is Way.SYNC:
            await anyio.to_thread.run_sync(scoped_sync)
        else:
            await scoped_async()
    unwound = raised.value

    with client_case.check("close() fails with the refused abort's error class"):
        assert isinstance(closed, RunStateUnavailableError)
        assert (closed.code, closed.group) == ("run_state_unavailable", "unavailable")
    with client_case.check(
        "the scoped form raises the close's error, chained to the program's"
    ):
        assert isinstance(unwound, RunStateUnavailableError)
        assert (unwound.__context__, unwound.__suppress_context__) == (thrown, False)
    with client_case.check(
        "both scripts played to their end, so no engine wrote a failure"
    ):
        assert engine_warnings(caplog) == []
