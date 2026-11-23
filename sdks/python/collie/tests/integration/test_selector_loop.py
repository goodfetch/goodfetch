"""The async client starts its Attached engine under asyncio's selector loop (ADR 0081).

Windows's `asyncio.SelectorEventLoop` cannot start a subprocess, so the
async client starts the engine with `subprocess.Popen` on a worker thread
of its own and bridges its pipes to the loop through threads; it never
sets an event-loop policy, which the test reads before and after. Off
Windows, the loop's base class is the same loop without POSIX's
subprocess support, so the case plays on every OS, and the Release
suite's Windows cell plays it on Windows's own loop.
"""

from __future__ import annotations

import asyncio
import asyncio.selector_events
import sys
import warnings
from typing import TYPE_CHECKING

import anyio
import pytest
from collie_calls import engine_warnings
from goodfetch_collie import AsyncCollie
from goodfetch_test_support import ClientCaseRecord, TestEnv, scripted_engine_env

if TYPE_CHECKING:
    from goodfetch_collie import RunReport
    from goodfetch_test_support import Product as ToolProduct

SCRIPT = "sdk-python/run/started-and-finished.json"
"""The script the engine plays: a Run started, then finished."""

PLAY_DEADLINE_SECONDS = 30
"""How long connecting, the Run and closing may take before the test fails."""


def selector_loop() -> asyncio.AbstractEventLoop:
    """The selector loop of asyncio as Windows has it, which starts no subprocess.

    On Windows, `asyncio.SelectorEventLoop` itself; elsewhere its base
    class, whose subprocess support only POSIX's subclass adds.
    """
    if sys.platform == "win32":
        return asyncio.SelectorEventLoop()
    return asyncio.selector_events.BaseSelectorEventLoop()


def event_loop_policy() -> object:
    """The event-loop policy asyncio holds now.

    Python 3.14 deprecates reading it, but no other read tells whether a
    program set one, which is what the story rules out.
    """
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", DeprecationWarning)
        return asyncio.get_event_loop_policy()  # type: ignore[deprecated]


@pytest.mark.covers("story:11-58")
@pytest.mark.client_case(id="selector-loop-starts-the-engine", mode="attached")
def test_the_async_client_starts_its_engine_under_the_selector_loop(
    test_env: TestEnv,
    product: ToolProduct,
    client_case: ClientCaseRecord,
    caplog: pytest.LogCaptureFixture,
) -> None:
    """Connected, a Run started and finished, closed; no policy set meanwhile."""
    env = scripted_engine_env(test_env, product, SCRIPT)

    async def played() -> RunReport:
        with anyio.fail_after(PLAY_DEADLINE_SECONDS):
            collie = await AsyncCollie.connect(env=env)
            run = await collie.start()
            report = await run.finish()
            await collie.close()
            return report

    policy = event_loop_policy()
    report = anyio.run(
        played, backend="asyncio", backend_options={"loop_factory": selector_loop}
    )

    with client_case.check(
        "under a loop that cannot start a subprocess, the async client started"
        " the Attached engine, which played its script to its end"
    ):
        assert report.lifecycle == "finished"
        assert engine_warnings(caplog) == []
    with client_case.check("the event-loop policy read after is the one read before"):
        assert event_loop_policy() is policy
