"""Which engine `connect()` reaches when the Daemon of the same commit runs (ADR 0024).

With `engine` at `auto`, the running Daemon; with `attached`, an Attached
engine beside it. `test_engine_choice.py` plays the other choices on the
Scripted engine. TypeScript's `engine-choice.test.ts` is its twin.
"""

from collections.abc import Mapping

import pytest
from collie_calls import open_collie
from goodfetch_collie import Product
from goodfetch_test_support import Daemon, TestEnv, Way, three_ways


@pytest.fixture
def with_daemon() -> bool:
    """The module's tests run beside the Daemon of the test's home."""
    return True


@pytest.mark.covers("story:11-2", "story:11-4")
@three_ways
async def test_reaches_the_running_daemon_at_auto_and_an_attached_engine_when_asked(
    way: Way, env: Mapping[str, str], daemon: Daemon, test_env: TestEnv
) -> None:
    """`auto` takes the running Daemon; `attached` starts an engine of its own."""
    on_daemon = await open_collie(way, env)
    attached = await open_collie(way, env, engine="attached")
    engines = [on_daemon.engine, attached.engine]
    await on_daemon.close()
    await attached.close()

    assert [
        (engine.kind, engine.product, engine.home, engine.engine_path, engine.url)
        for engine in engines
    ] == [
        ("daemon", Product.collie, str(test_env.home), None, daemon.socket),
        (
            "attached",
            Product.collie,
            str(test_env.home),
            env["GOODFETCH_COLLIE_ENGINE_PATH"],
            None,
        ),
    ]


@pytest.mark.covers("story:3-2")
@three_ways
async def test_two_scripts_open_at_once_share_the_running_daemon(
    way: Way, env: Mapping[str, str], daemon: Daemon, test_env: TestEnv
) -> None:
    """One script finds the Daemon running, the other has it configured.

    Both stay open at once on the one Daemon, and neither starts an engine.
    """
    found = await open_collie(way, env)
    configured = await open_collie(way, env, engine="daemon")
    engines = [found.engine, configured.engine]
    await found.close()
    await configured.close()

    assert [
        (engine.kind, engine.product, engine.home, engine.engine_path, engine.url)
        for engine in engines
    ] == [("daemon", Product.collie, str(test_env.home), None, daemon.socket)] * 2
