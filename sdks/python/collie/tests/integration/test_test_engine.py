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
import sys
from pathlib import Path
from typing import TYPE_CHECKING

import anyio.to_thread
import pytest
from collie_calls import failure, response_on
from goodfetch_collie import (
    ArtifactUnavailableOfflineError,
    AsyncCollie,
    EgressRefusedError,
    Settings,
    testing,
)
from goodfetch_test_support import BrowserDownload, ProvingGround, Way, three_ways

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_collie_client import testing as client_testing

PUBLIC_HOST = "public.test"
"""A host the test's `network.resolve` maps to an address that is not loopback.

TEST-NET-1's, which no Run reaches.
"""

Entry = tuple[bool, int, int]
"""An entry of a Browser cache: whether it is a folder, its length and mtime."""


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


def tree(cache: Path) -> dict[str, Entry]:
    """Every entry of the Browser cache at `cache`, by its path under `cache`.

    Its kind, length and modification time, symbolic links unfollowed: the
    cache's own records, and not a build's `files` folder, where no engine
    writes and the browser it launches keeps its own folders.
    """
    entries: dict[str, Entry] = {}
    for folder, folders, files in os.walk(cache):
        here = Path(folder)
        folders[:] = [
            name
            for name in folders
            if not (name == "files" and len(here.relative_to(cache).parts) == 3)
        ]
        for name in [*folders, *files]:
            path = here / name
            status = path.lstat()
            entries[path.relative_to(cache).as_posix()] = (
                path.is_dir() and not path.is_symlink(),
                status.st_size,
                status.st_mtime_ns,
            )
    return entries


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


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground's own seed for these tests."""
    return 3398


@pytest.mark.skipif(
    sys.platform == "win32", reason="the cache is made read-only by its folder's mode"
)
@pytest.mark.parametrize("with_browser", [True])
@three_ways
async def test_a_loopback_resolve_entry_is_reached_any_other_host_is_refused(
    way: Way, env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """Live through a loopback `resolve` entry; refused elsewhere; the cache unchanged.

    The caller's Browser cache is read-only here, as a prepared image's is:
    an engine records a build's use in a cache it can write (ADR 0098).
    """
    cache = Path(env["GOODFETCH_HOME"]) / "browsers"
    resolve = {host: [address] for host, address in proving_ground.resolve.items()}
    resolve[PUBLIC_HOST] = ["192.0.2.7"]
    reached = await anyio.to_thread.run_sync(proving_ground.start_url, "statuses/1")
    before = tree(cache)
    cache.chmod(0o555)
    try:
        engine = await start(way, env, resolve)
        run = await connection_of(way, engine).start()
        page = await run.new_page()

        navigated = await page.goto(reached)
        assert navigated is not None, "a document has a response"
        status = response_on(way, navigated).status
        refused = await failure(page.goto(proving_ground.url(PUBLIC_HOST, "/")))
        await run.finish()
        await engine.close()
    finally:
        cache.chmod(0o755)

    assert (status, type(refused)) == (200, EgressRefusedError)
    assert tree(cache) == before


@pytest.mark.parametrize("browser_download", [BrowserDownload.NOT_FOUND])
@three_ways
async def test_with_no_browser_installed_the_first_page_fails_offline(
    way: Way, env: Mapping[str, str]
) -> None:
    """`artifact_unavailable_offline`, its `fix` naming `browser install`.

    The caller's empty Browser cache stays empty.
    """
    cache = Path(env["GOODFETCH_BROWSERS_PATH"])
    engine = await start(way, env)
    run = await connection_of(way, engine).start()
    refused = await failure(run.new_page())
    await run.finish()
    await engine.close()

    assert isinstance(refused, ArtifactUnavailableOfflineError)
    assert refused.fix == (
        "Run `goodfetch-collie browser install` beforehand where the Browser"
        " cache is writable, or lift `egress.offline` where it was set"
    )
    assert tree(cache) == {}
