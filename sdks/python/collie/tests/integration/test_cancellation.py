"""Cancellation (ADR 0080) on the Scripted engine, three ways.

A call the program cancels once the engine has read it sends `$/cancel`
naming it, without waiting, and the cancellation propagates unchanged: the
backend's cancellation exception for the async API, on asyncio and on Trio,
and Ctrl+C's `KeyboardInterrupt` for the sync API (ADR 0081), at once even
when the signal lands on another thread than the blocked caller, as on
Windows. With a
cancellation and a timeout both set, whichever comes first ends the call. A
click already sent is neither sent again nor undone. The script fails on a
message it does not expect and on a step left unused, and the engine writes
that failure to its stderr, which reaches the `goodfetch.client` logger at
warning. TypeScript's `cancellation.test.ts` and the Rust client crate's
`cancellation.rs` are its twins.
"""

from __future__ import annotations

import json
import os
import signal
import subprocess
import sys
import time
from datetime import timedelta
from pathlib import Path
from typing import TYPE_CHECKING

import anyio
import pytest
from collie_calls import engine_warnings, failure, open_collie
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_client_methods,
    scripted_engine_env,
    until_scripted_engine_read,
)

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable

    from goodfetch_collie import GoodfetchError
    from goodfetch_test_support import Product as ToolProduct

SCRIPT = "sdk-python/run/cancellation-reaches-the-engine.json"
"""The script every way plays."""

PROGRAMS = Path(__file__).parents[1] / "programs"
"""The folder of the programs the tests start."""

LATE_CANCEL_SECONDS = 60.0
"""When the cancellation armed beside a 2 ms timeout would come."""

PROGRAM_DEADLINE_SECONDS = 60.0
"""How long the sync host may take, its engine's start included."""

PROMPT_SECONDS = 1.0
"""How soon a Ctrl+C raised on another thread must end the sync call it interrupts."""

CLIENT_METHODS = [
    "hello",
    "run.start",
    "run.new_page",
    "locator.click",
    "$/cancel",
    "run.finish",
    "$/cancel",
    "run.finish",
    "run.finish",
]
"""What the script expects the Client to send, in its order."""


async def cancelled_once_read(
    call: Callable[[], Awaitable[object]], home: Path, method: str
) -> bool:
    """Whether cancelling `call` once the engine has read it propagated.

    The engine has read it once its record holds a call of `method`.
    """
    propagated = False

    async def calling() -> None:
        nonlocal propagated
        try:
            await call()
        except anyio.get_cancelled_exc_class():
            propagated = True
            raise

    async with anyio.create_task_group() as calls:
        calls.start_soon(calling)
        await until_scripted_engine_read(home, method)
        calls.cancel_scope.cancel()
    return propagated


@pytest.mark.covers("story:11-47")
@pytest.mark.client_case(id="cancellation-reaches-the-engine", mode="attached")
@pytest.mark.parametrize(
    ("anyio_backend", "way"),
    [
        pytest.param("asyncio", Way.ASYNCIO, id="asyncio"),
        pytest.param("trio", Way.TRIO, id="trio"),
    ],
)
@pytest.mark.anyio
async def test_a_cancelled_task_sends_cancel_and_the_first_of_it_and_the_timeout_wins(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """A task cancelled mid-call sends `$/cancel`; its cancellation propagates."""
    env = scripted_engine_env(test_env, product, SCRIPT)
    collie = await open_collie(way, env)
    run = await collie.start()
    page = await run.new_page()

    click = await cancelled_once_read(
        page.locator("#buy").click, test_env.home, "locator.click"
    )
    finish = await cancelled_once_read(
        lambda: run.finish(timeout=timedelta(seconds=60)),
        test_env.home,
        "run.finish",
    )
    timed_out: GoodfetchError | None = None
    with anyio.move_on_after(LATE_CANCEL_SECONDS) as late_cancel:
        timed_out = await failure(run.finish(timeout=timedelta(milliseconds=2)))
    report = await run.finish()
    await collie.close()

    with client_case.check(
        "a cancelled click propagates the backend's cancellation, never call_cancelled"
    ):
        assert click
    with client_case.check(
        "a cancellation that comes before the timeout ends the call, propagating"
    ):
        assert finish
    with client_case.check(
        "a timeout that comes first ends the call with deadline_exceeded"
    ):
        assert not late_cancel.cancelled_caught
        assert timed_out is not None
        assert timed_out.code == "deadline_exceeded"
    with client_case.check(
        "each cancellation sent one $/cancel the engine read, and the click was"
        " neither sent again nor undone"
    ):
        assert scripted_engine_client_methods(test_env.home) == CLIENT_METHODS
    with client_case.check("the next call goes as usual"):
        assert report.lifecycle == "finished"
    with client_case.check(
        "the script played to its end, so its engine wrote no failure"
    ):
        assert engine_warnings(caplog) == []


def interrupt_once_read(host: subprocess.Popen[bytes], home: Path, method: str) -> None:
    """Sends the host Ctrl+C's SIGINT once its engine has read one call of `method`."""
    deadline = time.monotonic() + PROGRAM_DEADLINE_SECONDS
    while method not in scripted_engine_client_methods(home):
        assert host.poll() is None, f"the host ended before its {method} was read"
        assert time.monotonic() < deadline, f"the engine never read {method}"
        time.sleep(0.01)
    os.kill(host.pid, signal.SIGINT)


@pytest.mark.covers("story:11-47")
@pytest.mark.client_case(id="cancellation-reaches-the-engine", mode="attached")
def test_ctrl_c_in_a_sync_call_sends_cancel_and_propagates_keyboard_interrupt(
    test_env: TestEnv, product: ToolProduct, client_case: ClientCaseRecord
) -> None:
    """The sync API's way: Ctrl+C in the calling thread cancels its call.

    Only the main thread receives `KeyboardInterrupt`, so the sync calls
    run in a host program's main thread, which the test signals.
    """
    if sys.platform == "win32":
        pytest.skip("its signal is SIGINT; Windows's CTRL_C_EVENT has no test yet")
    env = scripted_engine_env(test_env, product, SCRIPT)
    out = test_env.folder / "cancelled.json"
    host = subprocess.Popen(
        [sys.executable, str(PROGRAMS / "collie_cancelled.py"), str(out)],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        cwd=test_env.folder,
        env=dict(env),
        start_new_session=True,
    )
    try:
        interrupt_once_read(host, test_env.home, "locator.click")
        interrupt_once_read(host, test_env.home, "run.finish")
        stdout, stderr = host.communicate(timeout=PROGRAM_DEADLINE_SECONDS)
    finally:
        if host.poll() is None:
            host.kill()
            host.communicate()

    with client_case.check("the host played its calls and exited 0"):
        assert host.returncode == 0, stderr.decode(errors="replace")
        seen = json.loads(out.read_text("utf-8"))
        client_case.message(seen)
    with client_case.check(
        "Ctrl+C ended the click and the finish whose timeout had not come, each"
        " with KeyboardInterrupt, never call_cancelled"
    ):
        assert (seen["click_interrupted"], seen["finish_interrupted"]) == (True, True)
    with client_case.check(
        "a timeout that comes first ends the call with deadline_exceeded"
    ):
        assert seen["timed_out"] == "deadline_exceeded"
    with client_case.check(
        "each Ctrl+C sent one $/cancel the engine read, and the click was"
        " neither sent again nor undone"
    ):
        assert scripted_engine_client_methods(test_env.home) == CLIENT_METHODS
    with client_case.check(
        "the next call goes as usual, and the script played to its end"
    ):
        assert seen["lifecycle"] == "finished"
        assert seen["warnings"] == []
    with client_case.check("the SDK wrote nothing to the host's stdout or stderr"):
        assert (stdout, stderr) == (b"", b"")


@pytest.mark.covers("story:11-56")
@pytest.mark.client_case(id="sync-calls-interrupt-from-any-thread", mode="attached")
def test_ctrl_c_on_another_thread_ends_a_blocked_sync_call_promptly(
    test_env: TestEnv, product: ToolProduct, client_case: ClientCaseRecord
) -> None:
    """Windows's way: the SIGINT lands on another thread than the blocked caller.

    The host raises it from a thread of its own, as Windows's console does
    for Ctrl+C, so the blocked main thread runs the signal's handler only
    once its wait returns: on every OS, the sync call's wait is timed, so
    the interrupt still ends the call at once.
    """
    env = scripted_engine_env(test_env, product, SCRIPT)
    out = test_env.folder / "cancelled.json"
    host = subprocess.Popen(
        [
            sys.executable,
            str(PROGRAMS / "collie_cancelled.py"),
            str(out),
            str(test_env.home),
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        cwd=test_env.folder,
        env=dict(env),
    )
    try:
        stdout, stderr = host.communicate(timeout=PROGRAM_DEADLINE_SECONDS)
    finally:
        if host.poll() is None:
            host.kill()
            host.communicate()

    with client_case.check("the host played its calls and exited 0"):
        assert host.returncode == 0, stderr.decode(errors="replace")
        seen = json.loads(out.read_text("utf-8"))
        client_case.message(seen)
    with client_case.check(
        "each Ctrl+C raised on another thread ended its blocked call with"
        " KeyboardInterrupt, within a second"
    ):
        assert (seen["click_interrupted"], seen["finish_interrupted"]) == (True, True)
        assert len(seen["interrupt_seconds"]) == 2
        assert all(seconds < PROMPT_SECONDS for seconds in seen["interrupt_seconds"])
    with client_case.check(
        "each Ctrl+C sent one $/cancel the engine read, and the next calls went"
        " as usual"
    ):
        assert scripted_engine_client_methods(test_env.home) == CLIENT_METHODS
        assert seen["timed_out"] == "deadline_exceeded"
        assert seen["lifecycle"] == "finished"
        assert seen["warnings"] == []
    with client_case.check("the SDK wrote nothing to the host's stdout or stderr"):
        assert (stdout, stderr) == (b"", b"")
