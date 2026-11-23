"""Collie's testing kit (issue 3398, ADR 0105), on the real engine.

`AsyncTestEngine` and `TestEngine` start an Attached engine of Collie's
executable of the same commit, three ways: async on asyncio, async on Trio
and through the sync `TestEngine`. It runs in a temporary Goodfetch home it
removes on close, with a cleared environment and `egress.offline` on; a host
`network.resolve` maps to loopback alone is reached live and any other is
refused, and browsers come from the caller's Browser cache, which the engine
reads and never writes. Rust's `tests/it/sdk/test_engine.rs` is its twin.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import TYPE_CHECKING

import anyio.to_thread
from goodfetch_collie import AsyncCollie, Settings, testing
from goodfetch_test_support import Way, three_ways

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_collie_client import testing as client_testing


async def start(
    way: Way,
    env: Mapping[str, str],
    resolve: Mapping[str, list[str]] | None = None,
) -> client_testing.AsyncTestEngine[AsyncCollie]:
    """A test engine this way started with `env` and `network.resolve`'s `resolve`."""
    settings: Settings = {} if resolve is None else {"network": {"resolve": resolve}}
    return await way.open(
        lambda: testing.AsyncTestEngine.start(env=env, settings=settings),
        lambda: testing.TestEngine.start(env=env, settings=settings),
    )


def connection_of(
    way: Way, engine: client_testing.AsyncTestEngine[AsyncCollie]
) -> AsyncCollie:
    """`engine`'s connection, its methods async whatever the way.

    It is a property, so the sync way's proxy hands the sync `Collie` itself,
    which `way.api` wraps.
    """
    connection = engine.connection
    return way.api(connection, connection)


@three_ways
async def test_a_test_runs_its_code_on_the_connection_and_close_removes_its_home(
    way: Way, env: Mapping[str, str]
) -> None:
    """The Run finishes on `engine.connection`; its home is gone after `close()`."""
    engine = await start(way, env)
    connection = connection_of(way, engine)
    home = connection.engine.home
    assert home is not None, "an Attached engine names its home"
    existed = await anyio.to_thread.run_sync(os.path.isdir, home)

    run = await connection.start()
    report = await run.finish()
    await engine.close()

    assert report.lifecycle == "finished"
    assert Path(home) != Path(env["GOODFETCH_HOME"])
    assert (existed, await anyio.to_thread.run_sync(os.path.exists, home)) == (
        True,
        False,
    ), home
