"""A page's and a Session's listeners on the Scripted engine (ADR 0081).

Each test plays a script of the shared corpus over the in-memory transport,
which fails the test on a message it does not expect and on a step left
unused, three ways: async on asyncio, async on Trio and through the sync
`Collie`. `on` sends its call when called, in call order, with no queue the
program sees; a listener that throws is removed, and `listener_failed` is
raised once, by the next call on its value or by the `close()` that closes
the value or the Run above it, or logged at error when the value was
released first. A Session's events come on the Connection's own
Subscription, which its listeners share, rather than on their calls.
TypeScript's `events.test.ts` is its twin.
"""

import gc
import logging
from collections.abc import Awaitable, Callable, Mapping
from typing import TYPE_CHECKING, cast

import anyio
import anyio.lowlevel
import pytest
from goodfetch_collie import (
    AsyncCollie,
    ClosedError,
    GoodfetchError,
    ListenerFailedError,
)
from goodfetch_collie.testing import async_connect_over, connect_over
from goodfetch_collie_client.generated.models import ConsoleMessage
from goodfetch_foxhound.generated.errors import AttemptEndedError
from goodfetch_test_support import (
    Finished,
    PlayedMessage,
    PlayedScript,
    Way,
    play_script,
    three_ways,
)

if TYPE_CHECKING:
    from goodfetch_collie_client.generated.handles import AsyncPage

CLIENT_LOGGER = "goodfetch.client"
"""The logger a failed listener nothing could raise goes to."""


async def collie_on(way: Way, script: str) -> tuple[AsyncCollie, PlayedScript]:
    """The Scripted engine playing `script`, and a Connection this way opened on it."""
    played = play_script(f"sdk-python/{script}")
    collie = await way.open(
        lambda: async_connect_over(played.transport),
        lambda: connect_over(played.transport),
    )
    return collie, played


async def failure(call: Awaitable[object]) -> GoodfetchError:
    """The Goodfetch error `call` raises."""
    with pytest.raises(GoodfetchError) as raised:
        await call
    return raised.value


async def until(holds: Callable[[], bool], what: str) -> None:
    """Waits until `holds()` is true, for 5 seconds at most.

    A listener runs on its own task, or on a worker thread in the sync way,
    and sets no event the test could wait on, so the wait polls it.
    """
    for _ in range(5000):
        if holds():
            return
        await anyio.sleep(0.001)
    message = f"{what} did not happen within 5 seconds"
    raise AssertionError(message)


def sent(played: PlayedScript, method: str) -> bool:
    """Whether the Client has sent a message for `method`."""
    return times_sent(played, method) > 0


def times_sent(played: PlayedScript, method: str) -> int:
    """How many messages the Client has sent for `method`."""
    return sum(
        isinstance(played_message, PlayedMessage)
        and played_message.from_ == "client"
        and isinstance(message := played_message.message, Mapping)
        and cast("Mapping[str, object]", message).get("method") == method
        for played_message in played.messages
    )


class Throwing:
    """A listener that throws on its first message, and whether it did."""

    def __init__(self, bug: BaseException | None = None) -> None:
        """Has not thrown yet; throws `bug`, or a `ValueError`."""
        self.bug = ValueError("the listener's bug") if bug is None else bug
        self.threw = False

    def __call__(self, _: object) -> None:
        """Throws the bug."""
        self.threw = True
        raise self.bug


@pytest.mark.covers("story:11-61", "story:11-62", "story:11-63")
@three_ways
async def test_sends_each_on_when_called_in_call_order_and_closes_by_cancelling_it(
    way: Way,
) -> None:
    """Each `on` reaches the engine with its credits; closing sends `$/cancel`."""
    collie, played = await collie_on(way, "page/listened.json")
    run = await collie.start()
    page = await run.new_page()
    texts: list[str] = []

    heard = page.on("console", lambda message: texts.append(message.text))
    with page.on("pageerror", lambda _: None):
        await until(lambda: texts == ["first", "second"], "both messages")
        heard.close()
    await run.finish()
    await collie.close()

    assert played.done() == Finished()


@pytest.mark.covers("story:11-63")
@pytest.mark.anyio
async def test_awaits_what_a_listener_returns_before_the_next_event() -> None:
    """One event at a time, in order: the second waits for the first's awaitable."""
    played = play_script("sdk-python/page/listened.json")
    collie = await async_connect_over(played.transport)
    run = await collie.start()
    page = await run.new_page()
    steps: list[str] = []

    async def slow(message: ConsoleMessage) -> None:
        steps.append(f"{message.text} began")
        for _ in range(5):
            await anyio.lowlevel.checkpoint()
        steps.append(f"{message.text} ended")

    heard = page.on("console", slow)
    with page.on("pageerror", lambda _: None):
        await until(lambda: len(steps) == 4, "both messages")
        heard.close()
    await run.finish()
    await collie.close()

    assert steps == ["first began", "first ended", "second began", "second ended"]
    assert played.done() == Finished()


@pytest.mark.covers("story:11-62")
@pytest.mark.anyio
async def test_closing_its_page_cancels_and_awaits_its_running_listener() -> None:
    """Once the page's close returns, none of its listeners runs; nothing is sent."""
    played = play_script("sdk-python/page/listener-stopped-on-close.json")
    collie = await async_connect_over(played.transport)
    run = await collie.start()
    page = await run.new_page()
    steps: list[str] = []

    async def waiting(message: ConsoleMessage) -> None:
        steps.append(f"{message.text} began")
        try:
            await anyio.sleep_forever()
        finally:
            steps.append(f"{message.text} cancelled")

    page.on("console", waiting)
    await until(lambda: steps == ["first began"], "the listener's start")
    await page.close()
    once_closed = list(steps)
    await run.finish()
    await collie.close()

    assert once_closed == ["first began", "first cancelled"]
    assert played.done() == Finished()


@pytest.mark.covers("story:11-62")
@pytest.mark.anyio
async def test_refuses_on_outside_an_event_loop_and_holds_back_no_later_call() -> None:
    """`on` on a thread with no loop raises, and the Run's next call still goes."""
    played = play_script("sdk-python/page/on-outside-a-loop.json")
    collie = await async_connect_over(played.transport)
    run = await collie.start()
    page = await run.new_page()

    with pytest.raises(RuntimeError):
        await anyio.to_thread.run_sync(lambda: page.on("console", lambda _: None))
    with anyio.fail_after(5):
        await run.finish()
    await collie.close()

    assert played.done() == Finished()


@pytest.mark.covers("story:11-63", "error:listener_failed")
@three_ways
async def test_raises_a_thrown_listeners_failure_once_from_the_next_call_unsent(
    way: Way,
) -> None:
    """The next call on its page raises `listener_failed`, chained to the bug."""
    collie, played = await collie_on(way, "page/listener-failed.json")
    run = await collie.start()
    page = await run.new_page()
    failing = Throwing()

    page.on("console", failing)
    await until(lambda: sent(played, "$/cancel"), "the listener's removal")
    refused = await failure(page.evaluate("1"))
    await page.close()
    await run.finish()
    await collie.close()

    assert failing.threw
    assert isinstance(refused, ListenerFailedError)
    assert (refused.code, refused.group, refused.details) == (
        "listener_failed",
        "program",
        {"event": "page.console"},
    )
    assert refused.__cause__ is failing.bug
    assert played.done() == Finished()


@pytest.mark.covers("story:11-63", "error:listener_failed")
@three_ways
async def test_raises_a_listeners_attempt_ended_as_its_failure(way: Way) -> None:
    """`attempt_ended`, outside `Exception` (ADR 0093), fails a listener as any bug."""
    collie, played = await collie_on(way, "page/listener-failed.json")
    run = await collie.start()
    page = await run.new_page()
    failing = Throwing(
        AttemptEndedError("Attempt 1 of crawl request `7` already ended.")
    )

    page.on("console", failing)
    await until(lambda: sent(played, "$/cancel"), "the listener's removal")
    refused = await failure(page.evaluate("1"))
    await page.close()
    await run.finish()
    await collie.close()

    assert isinstance(refused, ListenerFailedError)
    assert refused.__cause__ is failing.bug
    assert played.done() == Finished()


@pytest.mark.covers("story:11-63", "error:listener_failed")
@three_ways
async def test_raises_a_thrown_listeners_failure_from_its_pages_close_once_closed(
    way: Way,
) -> None:
    """The page's close closes it, then raises; a later call fails `closed`."""
    collie, played = await collie_on(way, "page/listener-failed-on-close.json")
    run = await collie.start()
    page = await run.new_page()
    failing = Throwing()

    page.on("console", failing)
    await until(lambda: sent(played, "$/cancel"), "the listener's removal")
    closing = await failure(page.close())
    after = await failure(page.evaluate("1"))
    await page.close()
    await run.finish()
    await collie.close()

    assert isinstance(closing, ListenerFailedError)
    assert closing.__cause__ is failing.bug
    assert isinstance(after, ClosedError)
    assert played.done() == Finished()


@pytest.mark.covers("story:11-63", "error:listener_failed")
@three_ways
async def test_raises_a_thrown_listeners_failure_from_its_runs_close_once_closed(
    way: Way,
) -> None:
    """The Run's close, a parent of the page, stops the Run, then raises."""
    collie, played = await collie_on(way, "page/listener-failed-on-run-close.json")
    run = await collie.start()
    page = await run.new_page()
    failing = Throwing()

    page.on("console", failing)
    await until(lambda: sent(played, "$/cancel"), "the listener's removal")
    closing = await failure(run.close())
    await run.close()
    await collie.close()

    assert isinstance(closing, ListenerFailedError)
    assert closing.__cause__ is failing.bug
    assert played.done() == Finished()


@pytest.mark.covers("story:11-63", "error:listener_failed")
@three_ways
async def test_logs_a_thrown_listeners_failure_at_error_once_its_page_is_released(
    way: Way, caplog: pytest.LogCaptureFixture
) -> None:
    """No call could raise it: the failure is logged, and no close raises it."""
    collie, played = await collie_on(way, "page/listener-failed-on-run-close.json")
    run = await collie.start()
    page = await run.new_page()
    failing = Throwing()

    with caplog.at_level(logging.ERROR, CLIENT_LOGGER):
        page.on("console", failing)
        del page
        gc.collect()
        await until(lambda: sent(played, "$/cancel"), "the listener's removal")
        await run.close()
        await collie.close()

    [logged] = [record for record in caplog.records if record.name == CLIENT_LOGGER]
    assert logged.levelno == logging.ERROR
    assert logged.getMessage() == (
        "page.on: the listener of `page.console` threw, so it was removed"
    )
    assert logged.exc_info is not None
    assert isinstance(logged.exc_info[1], ListenerFailedError)
    assert logged.exc_info[1].__cause__ is failing.bug
    assert played.done() == Finished()


@pytest.mark.covers("story:11-61", "story:11-62")
@three_ways
async def test_a_sessions_listeners_share_one_subscription_and_hear_their_own_pages(
    way: Way,
) -> None:
    """Each hears only its Session's page, a working page; the last out cancels it."""
    collie, played = await collie_on(way, "page/session-listened.json")
    run = await collie.start()
    first = await run.new_session()
    second = await run.new_session()
    firsts: list[AsyncPage] = []
    seconds: list[AsyncPage] = []

    heard_first = first.on("page", firsts.append)
    heard_second = second.on("page", seconds.append)
    await until(
        lambda: (
            (len(firsts), len(seconds), times_sent(played, "$/credit")) == (1, 1, 2)
        ),
        "each Session's page, and the credits granted again",
    )
    values = [
        (await way.api(page, page).evaluate("() => 1")).value
        for page in (*firsts, *seconds)
    ]
    heard_first.close()
    heard_second.close()
    with first.on("page", firsts.append):
        await until(
            lambda: times_sent(played, "session.on") == 3, "the next Subscription"
        )
    await run.finish()
    await collie.close()

    assert values == [1, 2]
    assert played.done() == Finished()


@pytest.mark.covers("story:11-63", "error:listener_failed")
@three_ways
async def test_raises_a_thrown_session_listeners_failure_once_from_the_next_call(
    way: Way,
) -> None:
    """The next call on its Session raises `listener_failed` and is not sent."""
    collie, played = await collie_on(way, "page/session-listener-failed.json")
    run = await collie.start()
    session = await run.new_session()
    failing = Throwing()

    session.on("page", failing)
    await until(lambda: sent(played, "$/cancel"), "the listener's removal")
    refused = await failure(session.pages())
    await session.close()
    await run.finish()
    await collie.close()

    assert failing.threw
    assert isinstance(refused, ListenerFailedError)
    assert (refused.code, refused.group, refused.details) == (
        "listener_failed",
        "program",
        {"event": "session.page"},
    )
    assert refused.__cause__ is failing.bug
    assert played.done() == Finished()
