"""A Connection meets another process: a forked child, and Ctrl+C (ADRs 0078, 0093).

A Connection and every value under it belong to the process that opened
them, so a child created by `fork` that uses one fails with
`connection_forked` and writes nothing to the parent's engine, while it may
connect on its own. Ctrl+C sent to the host's process group never reaches
its Attached engine, which runs in a group of its own, and the host's
`KeyboardInterrupt` unwinds through its `with` blocks, which close the Run
with a Checkpoint, then the Connection. Both run the three ways, against
the Scripted engine. TypeScript's `interrupt.test.ts` is the interrupt's
twin; Node has no `fork`.
"""

from __future__ import annotations

import functools
import json
import os
import select
import signal
import subprocess
import sys
import time
import warnings
from pathlib import Path
from typing import TYPE_CHECKING

import anyio
import anyio.to_thread
import pytest
from collie_calls import engine_warnings, open_collie
from collie_tracer import start_and_finish
from goodfetch_collie import AsyncCollie, AsyncRun, GoodfetchError
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_client_methods,
    scripted_engine_env,
    three_ways,
)

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Mapping

    from goodfetch_test_support import Product as ToolProduct

PROGRAMS = Path(__file__).parents[1] / "programs"
"""The folder of the programs the tests start."""

PROGRAM_DEADLINE_SECONDS = 60.0
"""How long a program or a forked child may take, its engine's start included."""

FORK_FIX = "connect in the child process"
"""What `connection_forked`'s fix says to do, in its own words."""


def _seen(error: BaseException | None) -> list[object] | None:
    """What a child reports of the error a call raised: none when it passed."""
    if error is None:
        return None
    if not isinstance(error, GoodfetchError):
        return [type(error).__name__, str(error)]
    return [
        type(error).__name__,
        error.code,
        error.group,
        error.retryable,
        FORK_FIX in (error.fix or "").lower(),
    ]


async def _outcome(call: Callable[[], Awaitable[object]]) -> list[object] | None:
    try:
        await call()
    except Exception as error:
        return _seen(error)
    return None


def _in_child(
    way: Way, collie: AsyncCollie, run: AsyncRun, child_env: Mapping[str, str]
) -> dict[str, object]:
    """What the forked child saw: its parent's values used, then its own Connection."""

    async def use() -> dict[str, object]:
        seen: dict[str, object] = {
            "start": await _outcome(collie.start),
            "run_finish": await _outcome(run.finish),
            "raw_call": await _outcome(lambda: collie.call("sample.echo", {})),
            "close": await _outcome(collie.close),
        }
        own = await open_collie(way, child_env)
        traced = await start_and_finish(own)
        seen["own_lifecycle"] = traced.report.lifecycle
        return seen

    backend = "asyncio" if way is Way.SYNC else way.value
    return anyio.run(use, backend=backend)


def _forked(work: Callable[[], dict[str, object]]) -> dict[str, object]:
    """Runs `work` in a child created by `fork` and returns what it reported.

    The child writes its report as JSON to a pipe and ends with `os._exit`,
    so nothing of the parent's runs again in it; a child silent past the
    deadline is killed, and so is the test.
    """
    reading, writing = os.pipe()
    with warnings.catch_warnings():
        # Python warns that forking a threaded process may deadlock the
        # child: here the child is the subject.
        warnings.simplefilter("ignore", DeprecationWarning)
        pid = os.fork()
    if pid == 0:
        os.close(reading)
        try:
            report: object = work()
        except BaseException as error:
            report = {"child_failed": repr(error)}
        os.write(writing, json.dumps(report).encode())
        os._exit(0)
    os.close(writing)
    data = b""
    try:
        while True:
            ready, _, _ = select.select([reading], [], [], PROGRAM_DEADLINE_SECONDS)
            if not ready:
                os.kill(pid, signal.SIGKILL)
                message = "the forked child reported nothing within its deadline"
                raise TimeoutError(message)
            chunk = os.read(reading, 65536)
            if not chunk:
                break
            data += chunk
    finally:
        os.close(reading)
        os.waitpid(pid, 0)
    reported: dict[str, object] = json.loads(data)
    return reported


@pytest.mark.client_case(id="connection-used-after-fork", mode="attached")
@three_ways
async def test_a_forked_child_fails_on_its_parents_connection_and_connects_its_own(
    way: Way,
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Every use in the child fails with `connection_forked`; its own Connection works.

    The parent's engine plays `closed-while-running.json`, which a single
    message from the child would fail, so its Run closes as if no child
    had been.
    """
    if not hasattr(os, "fork"):
        pytest.skip("Windows has no fork: no other process ever holds a Connection")
    parent_env = scripted_engine_env(
        test_env,
        product,
        "sdk-python/run/closed-while-running.json",
        test_env.folder / "parent",
    )
    child_env = scripted_engine_env(
        test_env,
        product,
        "sdk-python/run/started-and-finished.json",
        test_env.folder / "child",
    )
    collie = await open_collie(way, parent_env)
    run = await collie.start(run_id="nightly", tags=["shop"])

    # Forked from a worker thread, the child has no event loop of its parent's.
    seen = await anyio.to_thread.run_sync(
        _forked, functools.partial(_in_child, way, collie, run, child_env)
    )
    client_case.message(seen)
    await run.close()
    await collie.close()

    forked = ["ConnectionForkedError", "connection_forked", "program", False, True]
    with client_case.check(
        "a start, a Run's call, a raw call and the close each fail in the child"
        " with connection_forked, not retryable, whose fix says to connect in the"
        " child process"
    ):
        assert {
            key: seen.get(key) for key in ("start", "run_finish", "raw_call", "close")
        } == {
            "start": forked,
            "run_finish": forked,
            "raw_call": forked,
            "close": forked,
        }
    with client_case.check("a Connection the child opens works"):
        assert seen.get("own_lifecycle") == "finished"
    with client_case.check(
        "the child wrote nothing to the parent's engine, whose Run closed as if"
        " no child had been"
    ):
        assert scripted_engine_client_methods(test_env.folder / "parent") == [
            "hello",
            "run.start",
            "run.abort",
        ]
        assert engine_warnings(caplog) == []


@pytest.mark.client_case(id="interrupt-reaches-only-the-host", mode="attached")
@pytest.mark.parametrize("way", ["asyncio", "trio", "sync"])
def test_ctrl_c_reaches_only_the_host_whose_unwinding_closes_the_run(
    way: str, test_env: TestEnv, product: ToolProduct, client_case: ClientCaseRecord
) -> None:
    """SIGINT to the host's process group leaves its engine running.

    The host's `KeyboardInterrupt` closes its Run, which the engine stops
    with a Checkpoint, and its Connection. An engine in the host's group
    would die of the signal before the Run's close reached it.
    """
    if sys.platform == "win32":
        pytest.skip("its signal is SIGINT; Windows's CTRL_C_EVENT has no test yet")
    env = scripted_engine_env(
        test_env, product, "sdk-python/run/closed-while-running.json"
    )
    out = test_env.folder / "interrupted.json"
    ready = test_env.folder / "running"
    host = subprocess.Popen(
        [
            sys.executable,
            str(PROGRAMS / "collie_interrupted.py"),
            str(out),
            str(ready),
            way,
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        cwd=test_env.folder,
        env=dict(env),
        # The host's own group, as a terminal gives the program it runs.
        start_new_session=True,
    )
    try:
        deadline = time.monotonic() + PROGRAM_DEADLINE_SECONDS
        while not ready.exists():
            assert host.poll() is None, "the host ended before its Run ran"
            assert time.monotonic() < deadline, "the host's Run never ran"
            time.sleep(0.05)
        # Ctrl+C: the terminal signals the whole foreground group.
        os.killpg(host.pid, signal.SIGINT)
        stdout, stderr = host.communicate(timeout=PROGRAM_DEADLINE_SECONDS)
    finally:
        if host.poll() is None:
            host.kill()
            host.communicate()

    with client_case.check("the host unwound from KeyboardInterrupt and exited 0"):
        assert host.returncode == 0, stderr.decode(errors="replace")
        seen = json.loads(out.read_text("utf-8"))
        client_case.message(seen)
        assert seen["interrupted"] is True
    with client_case.check(
        "the engine, in a group of its own, outlived the signal, got the Run's"
        " close, run.abort, and played its script to its end"
    ):
        assert scripted_engine_client_methods(test_env.home) == [
            "hello",
            "run.start",
            "run.abort",
        ]
        assert seen["warnings"] == []
    with client_case.check("the SDK wrote nothing to the host's stdout or stderr"):
        assert (stdout, stderr) == (b"", b"")
