"""An async value belongs to the event loop that opened it (ADR 0081).

A call on an `AsyncCollie` or a Run under it from another event loop, on
another thread, fails at once with `event_loop_mismatch`, a `ProgramError`
whose fix names the sync `Collie`, and sends nothing, where it would
otherwise fail later with "the future belongs to a different loop" or
hang; the same call from the opening loop works. A sync `Collie`'s values
may be called from any thread, one running a loop of its own included.
The case plays three ways, as an Attached engine and as a Daemon, against
the Scripted engine, whose script holds each call once: a refused call
that reached it would fail the script. Only the Python runtime raises
`event_loop_mismatch`: no other SDK has an event loop a value belongs to.
"""

from __future__ import annotations

import functools
from typing import TYPE_CHECKING, TypeVar

import anyio
import anyio.to_thread
import pytest
from collie_calls import engine_warnings, exit_code, open_collie
from goodfetch_collie import AsyncCollie, EventLoopMismatchError
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    start_scripted_daemon,
    three_ways,
)
from goodfetch_test_support.ways import ASYNC_BACKENDS

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable

    from goodfetch_collie import AsyncRun, RunReport
    from goodfetch_test_support import Product as ToolProduct

COVERS = pytest.mark.covers("story:11-55", "error:event_loop_mismatch")

OTHER_LOOP_DEADLINE_SECONDS = 10
"""How long a call from another loop may take, failing or not, before the test fails."""

_T = TypeVar("_T")


async def from_another_loop(backend: str, call: Callable[[], Awaitable[_T]]) -> _T:
    """What `call` returns, awaited in a new `backend` loop on a thread of its own."""

    async def awaited() -> _T:
        with anyio.fail_after(OTHER_LOOP_DEADLINE_SECONDS):
            return await call()

    return await anyio.to_thread.run_sync(
        functools.partial(anyio.run, awaited, backend=backend)
    )


async def refused_from_other_loops(
    call: Callable[[], Awaitable[object]],
) -> list[EventLoopMismatchError]:
    """The error `call` raises from a new loop of each backend, each on its thread."""
    refused: list[EventLoopMismatchError] = []
    for backend in ASYNC_BACKENDS:
        with pytest.raises(EventLoopMismatchError) as raised:
            await from_another_loop(backend, call)
        refused.append(raised.value)
    return refused


async def played(
    way: Way, collie: AsyncCollie
) -> tuple[list[EventLoopMismatchError], RunReport]:
    """The refusals from other loops, then the Run report the script answers.

    An async value refuses each call from another loop, then answers it
    from its own; a sync value answers the same calls from those loops.
    """
    refused: list[EventLoopMismatchError] = []
    if way is Way.SYNC:
        run: AsyncRun = await from_another_loop("asyncio", collie.start)
        report = await from_another_loop("trio", run.finish)
    else:
        refused += await refused_from_other_loops(collie.start)
        run = await collie.start()
        refused += await refused_from_other_loops(run.finish)
        report = await run.finish()
    await collie.close()
    return refused, report


def check_played(
    client_case: ClientCaseRecord,
    way: Way,
    refused: list[EventLoopMismatchError],
    report: RunReport,
) -> None:
    """Each refusal names the sync `Collie`; the calls from the opening loop worked."""
    with client_case.check(
        "an async value refuses each call from another loop or thread with"
        " event_loop_mismatch, whose fix names the sync Collie; a sync value"
        " answers them"
    ):
        expected = (
            []
            if way is Way.SYNC
            else [("run.start", "program")] * 2 + [("run.finish", "program")] * 2
        )
        assert [(error.operation, error.group) for error in refused] == expected
        assert all(error.code == "event_loop_mismatch" for error in refused)
        assert all("`Collie`" in (error.fix or "") for error in refused)
    with client_case.check("the same calls from the opening loop worked"):
        assert report.lifecycle == "finished"


@COVERS
@pytest.mark.client_case(id="async-values-belong-to-their-loop", mode="attached")
@three_ways
async def test_an_async_value_refuses_another_loop_and_a_sync_one_takes_any_thread(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Refused from other loops, sending nothing; answered from the opening one."""
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/started-and-finished.json"
    )
    refused, report = await played(way, await open_collie(way, env))

    check_played(client_case, way, refused, report)
    with client_case.check(
        "the script played to its end, so no refused call reached the engine"
    ):
        assert engine_warnings(caplog) == []


@COVERS
@pytest.mark.client_case(id="async-values-belong-to-their-loop", mode="daemon")
@three_ways
async def test_an_async_value_refuses_another_loop_on_a_daemon_too(
    way: Way,
    test_env: TestEnv,
    client_case: ClientCaseRecord,
) -> None:
    """The same refusals and answers from a Daemon."""
    daemon = start_scripted_daemon(
        test_env, "sdk-python/run/started-and-finished-from-a-daemon.json", "socket"
    )
    collie = await open_collie(way, test_env.env, engine="daemon")
    refused, report = await played(way, collie)
    code = await exit_code(daemon.wait)

    check_played(client_case, way, refused, report)
    with client_case.check(
        "the Daemon played its script through, so no refused call reached it"
    ):
        assert code == 0
