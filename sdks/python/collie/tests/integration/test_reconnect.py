"""A remote Daemon's dropped link (spec 606, ADR 0079).

The Scripted engine, played as a Daemon over a WebSocket, drops the Client
with no close, then plays the rest of its script to the next Client that
connects. The Client reaches it again by itself, greets it with the same
hello, resubscribes after the last event it read, and sends again what it
was waiting for only when it is a read or an idempotent write; an Effect in
flight raises `connection_lost`. A Scripted Daemon exits 0 only once every
step played, so its exit says the Client sent exactly what the script holds
after the drop, the resubscription's position and credits included.
TypeScript's `reconnect.test.ts` is its twin.
"""

from collections.abc import Callable

import anyio
import pytest
from collie_calls import exit_code, failure, open_collie
from goodfetch_collie import ConnectionLostError, GoodfetchError
from goodfetch_collie_client.generated.connection import WEBSOCKET_SUBPROTOCOL
from goodfetch_test_support import (
    ClientCaseRecord,
    TestEnv,
    Way,
    scripted_engine_env,
    start_scripted_daemon,
    three_ways,
    until_scripted_engine_read,
)
from goodfetch_test_support import Product as ToolProduct

ATTACHED = "sdk-python/connect/reached-attached.json"
"""The script of an Attached engine, which a test reaching a Daemon never starts."""

TOKEN = "remote-daemon-token"
"""The token both scripts' `hello` carries."""

WAIT_SECONDS = 10.0
"""How long the test waits for a listener to hear the Session's pages."""

POLL_SECONDS = 0.01
"""How often the test looks whether the listener heard them."""


async def until(holds: Callable[[], bool], what: str) -> None:
    """Waits until `holds()` is true, for `WAIT_SECONDS` at most.

    A listener runs on its own task, or on a worker thread in the sync way,
    and sets no event the test could wait on, so the wait polls it.
    """
    for _ in range(round(WAIT_SECONDS / POLL_SECONDS)):
        if holds():
            return
        await anyio.sleep(POLL_SECONDS)
    message = f"{what} did not happen within {WAIT_SECONDS} seconds"
    raise AssertionError(message)


def remote_env(test_env: TestEnv, product: ToolProduct, address: str) -> dict[str, str]:
    """The environment naming the remote Daemon at `address` and its token."""
    return {
        **scripted_engine_env(test_env, product, ATTACHED),
        "GOODFETCH_URL": f"ws://{address}",
        "GOODFETCH_TOKEN": TOKEN,
    }


@pytest.mark.covers("story:11-6")
@three_ways
async def test_reaches_the_daemon_again_and_resubscribes_after_the_last_event_read(
    way: Way, test_env: TestEnv, product: ToolProduct
) -> None:
    """The listener loses no page: the second comes on the new link."""
    daemon = start_scripted_daemon(
        test_env,
        "sdk-python/connect/remote-drop-resubscribes.json",
        "websocket",
        subprotocol=WEBSOCKET_SUBPROTOCOL,
    )
    collie = await open_collie(way, remote_env(test_env, product, daemon.address))
    run = await collie.start()
    session = await run.new_session()
    pages: list[object] = []

    subscription = session.on("page", pages.append)
    await until(lambda: len(pages) == 2, "the Session's two pages")
    subscription.close()
    await run.finish()
    await collie.close()

    assert await exit_code(daemon.wait) == 0


@pytest.mark.covers("story:11-6", "story:11-32", "error:connection_lost")
@pytest.mark.client_case(id="dropped-link-resends-reads", mode="daemon")
@three_ways
async def test_sends_a_read_in_flight_again_and_raises_connection_lost_for_an_effect(
    way: Way, test_env: TestEnv, product: ToolProduct, client_case: ClientCaseRecord
) -> None:
    """The read is answered on the new link; the Effect is never sent again."""
    daemon = start_scripted_daemon(
        test_env,
        "sdk-python/connect/remote-drop-resends-a-read.json",
        "websocket",
        subprotocol=WEBSOCKET_SUBPROTOCOL,
        recorded=True,
    )
    collie = await open_collie(way, remote_env(test_env, product, daemon.address))
    run = await collie.start()
    lost: list[GoodfetchError] = []
    listed: list[object] = []

    async def effect() -> None:
        lost.append(await failure(run.new_page()))

    async with anyio.create_task_group() as group:
        group.start_soon(effect)
        # The script holds the Effect first: the read goes once it is read.
        await until_scripted_engine_read(test_env.home, "run.new_page")
        listed.append(await collie.call("run.list", {}))
    await run.finish()
    await collie.close()
    exit_status = await exit_code(daemon.wait)

    with client_case.check(
        "the read in flight is sent again and answered, and the Effect raises"
        " connection_lost"
    ):
        [error] = lost
        assert isinstance(error, ConnectionLostError)
        assert (error.code, error.group, error.operation) == (
            "connection_lost",
            "unavailable",
            "run.new_page",
        )
        assert listed == [{"runs": []}]
    with client_case.check(
        "the Daemon played its script through, so the Effect was never sent again"
    ):
        assert exit_status == 0
