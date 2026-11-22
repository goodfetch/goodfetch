"""Foxhound's Connection on the Scripted engine (ADR 0069).

Each test plays a script of the shared corpus over the in-memory transport,
as an Attached engine's stdio would carry it, so the hand-written runtime
runs with no process, three ways: async on asyncio, async on Trio and
through the sync `Foxhound`. The script fails the test on a message it does
not expect, `hello` included, and on a step left unused. Collie's
`test_collie.py` and TypeScript's `foxhound.test.ts` are its twins.
"""

from __future__ import annotations

import signal
from typing import TYPE_CHECKING

import pytest
from collie_tracer import start_and_finish
from connection_library import finish_one_run
from goodfetch_collie import AsyncCollie, Collie
from goodfetch_collie_client.connection import EngineInfo
from goodfetch_collie_client.connection_type import AsyncConnection, Connection
from goodfetch_collie_client.generated.errors import (
    NotInProductError,
    UnavailableError,
)
from goodfetch_collie_client.generated.models import Product, StartedAs
from goodfetch_collie_client.goodfetch_error import BaseGoodfetchError
from goodfetch_collie_client.run import AsyncRun, Run
from goodfetch_foxhound import AsyncFoxhound, Foxhound
from goodfetch_foxhound.generated.errors import ClientGoneError
from goodfetch_foxhound.testing import async_connect_over, connect_over
from goodfetch_test_support import (
    Closed,
    Finished,
    PlayedScript,
    Way,
    play_script,
    three_ways,
)

if TYPE_CHECKING:
    from collections.abc import Awaitable

# The signals a program's own handler may hold, which a Connection leaves be.
HANDLED_SIGNALS = [
    getattr(signal, name)
    for name in ("SIGINT", "SIGTERM", "SIGBREAK")
    if hasattr(signal, name)
]


async def foxhound_on(way: Way, script: str) -> tuple[AsyncFoxhound, PlayedScript]:
    """The Scripted engine playing `script`, and a Connection this way opened on it."""
    played = play_script(f"sdk-python/{script}")
    foxhound = await way.open(
        lambda: async_connect_over(played.transport),
        lambda: connect_over(played.transport),
    )
    return foxhound, played


async def failure(call: Awaitable[object]) -> BaseGoodfetchError:
    """The Goodfetch error `call` raises."""
    with pytest.raises(BaseGoodfetchError) as raised:
        await call
    return raised.value


def handlers() -> list[object]:
    """The process's handler of each signal a program may handle."""
    return [signal.getsignal(number) for number in HANDLED_SIGNALS]


@pytest.mark.covers("story:11-1002")
@three_ways
async def test_has_finished_hello_naming_foxhound_once_connect_returns(
    way: Way,
) -> None:
    """The Connection is open on a Foxhound engine, and says so, with no round trip."""
    foxhound, played = await foxhound_on(way, "connect/foxhound-accepted.json")

    engine = foxhound.engine
    await foxhound.close()

    assert engine == EngineInfo(
        kind="attached",
        product=Product.foxhound,
        version="0.0.0",
        protocol_version="0.1",
        project_file=None,
        home=None,
        engine_path=None,
        url=None,
        fallback=None,
    )
    assert played.done() == Finished()


@pytest.mark.covers("story:11-1002")
@three_ways
async def test_runs_beside_another_connection_installing_no_signal_handler(
    way: Way,
) -> None:
    """Two Connections open at once each finish a Run; no signal's handler changes."""
    before = handlers()
    first, first_played = await foxhound_on(
        way, "run/foxhound-started-and-finished.json"
    )
    second, second_played = await foxhound_on(
        way, "run/foxhound-started-and-finished.json"
    )
    while_open = handlers()

    traced = [await start_and_finish(first), await start_and_finish(second)]

    assert [trace.report.lifecycle for trace in traced] == ["finished", "finished"]
    assert [first_played.done(), second_played.done()] == [Finished(), Finished()]
    assert [while_open, handlers()] == [before, before]


@pytest.mark.covers("story:11-1002")
@three_ways
async def test_connects_again_after_a_close(way: Way) -> None:
    """A closed Connection leaves nothing behind that stops the next one."""
    first, first_played = await foxhound_on(way, "connect/foxhound-accepted.json")
    await first.close()

    second, second_played = await foxhound_on(
        way, "run/foxhound-started-and-finished.json"
    )
    traced = await start_and_finish(second)

    assert (traced.started_as, traced.report.lifecycle) == (StartedAs.new, "finished")
    assert [first_played.done(), second_played.done()] == [Finished(), Finished()]


def public_members(product: type[object]) -> set[str]:
    """The public members a Connection type carries, its own and inherited."""
    return {name for name in dir(product) if not name.startswith("_")}


@pytest.mark.covers("story:11-1003")
@pytest.mark.parametrize(
    ("base", "collie", "foxhound"),
    [
        pytest.param(AsyncConnection, AsyncCollie, AsyncFoxhound, id="async"),
        pytest.param(Connection, Collie, Foxhound, id="sync"),
    ],
)
def test_carries_every_member_of_collies_connection_under_the_same_name(
    base: type[object], collie: type[object], foxhound: type[object]
) -> None:
    """Foxhound's Connection extends the client package's, as Collie's does.

    It carries each of Collie's public members, `start` and `connect`
    included, and the scoped form's.
    """
    scoped = {"__aenter__", "__aexit__", "__enter__", "__exit__"}

    assert (
        issubclass(foxhound, base),
        public_members(collie) - public_members(foxhound),
        {name for name in scoped if hasattr(collie, name) != hasattr(foxhound, name)},
    ) == (True, set(), set())


@pytest.mark.covers("story:11-1003", "story:11-1004")
@three_ways
async def test_a_helper_typed_for_the_connection_takes_it_and_catches_its_error(
    way: Way,
) -> None:
    """A library's helper finishes a Run on it, then catches its `ClosedError`.

    `connection_library`'s helper takes the client package's
    `AsyncConnection` and catches the client package's `ClosedError`, as it
    does on Collie's Connection.
    """
    foxhound, played = await foxhound_on(way, "run/foxhound-started-and-finished.json")

    report = await finish_one_run(foxhound)
    await foxhound.close()
    answer = await finish_one_run(foxhound)

    assert not isinstance(report, str)
    assert (report.lifecycle, answer) == ("finished", "closed")
    assert played.done() == Finished()


@pytest.mark.covers("story:11-1004")
@three_ways
async def test_raises_the_class_of_a_code_only_foxhound_carries(way: Way) -> None:
    """Foxhound's own code raises its class, caught by the client package's group's."""
    foxhound, played = await foxhound_on(
        way, "run/foxhound-start-failed-with-a-foxhound-code.json"
    )

    error = await failure(foxhound.start())
    await foxhound.close()

    assert isinstance(error, ClientGoneError)
    assert isinstance(error, UnavailableError)
    assert (error.code, error.group, error.retryable) == (
        "client_gone",
        "unavailable",
        True,
    )
    assert played.done() == Finished()


@pytest.mark.covers("story:11-1004")
@pytest.mark.anyio
@pytest.mark.parametrize("anyio_backend", ["asyncio"])
async def test_hands_out_the_client_packages_own_runs() -> None:
    """A Run of either API is the client package's own type, as Collie's is."""
    async_played = play_script("sdk-python/run/foxhound-started-and-finished.json")
    sync_played = play_script("sdk-python/run/foxhound-started-and-finished.json")
    async_foxhound = await async_connect_over(async_played.transport)
    async_run = await async_foxhound.start()
    await async_run.finish()
    await async_foxhound.close()

    with connect_over(sync_played.transport) as foxhound:
        sync_run = foxhound.start()
        sync_run.finish()

    assert (type(async_run), type(sync_run)) == (AsyncRun, Run)
    assert [async_played.done(), sync_played.done()] == [Finished(), Finished()]


@pytest.mark.covers("story:11-1005")
@three_ways
async def test_fails_connect_on_a_collie_engine_with_not_in_product(way: Way) -> None:
    """A Collie engine refuses `hello`, which raises before any other call.

    The error's details name the Product needed, the engine's Product and
    version and the install command of the channel the Client came from,
    which its `fix` names.
    """
    played = play_script("sdk-python/connect/foxhound-refused-by-collie.json")

    error = await failure(
        way.open(
            lambda: async_connect_over(played.transport),
            lambda: connect_over(played.transport),
        )
    )

    assert isinstance(error, NotInProductError)
    assert (error.code, error.group, error.retryable) == (
        "not_in_product",
        "unavailable",
        False,
    )
    assert error.fix == (
        "Connect to a Foxhound engine, or run with Foxhound, which contains "
        "everything Collie does (`pip install goodfetch-foxhound`)."
    )
    assert error.details == {
        "needs": "foxhound",
        "product": "collie",
        "product_version": "0.0.0",
        "install": "pip install goodfetch-foxhound",
    }
    assert played.done() == Closed()
