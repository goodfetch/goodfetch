"""A page's listeners on an Attached engine (ADRs 0069 and 0081).

The engine is Collie's executable of the same commit, which launches the
browser of the test's own Browser cache, three ways: async on asyncio,
async on Trio and through the sync `Collie`. A listener gets its page's
console messages in order, and one that throws is removed, its
`listener_failed` raised once by the next call on its page, which is not
sent and leaves the page open. TypeScript's `events.test.ts` is its twin.
"""

from collections.abc import Awaitable, Callable, Mapping

import anyio
import pytest
from goodfetch_collie import AsyncCollie, Collie, GoodfetchError, ListenerFailedError
from goodfetch_collie_client.generated.models import ConsoleMessage
from goodfetch_test_support import ClientCaseRecord, Way, three_ways

WAIT_SECONDS = 30.0
"""How long the test waits for a listener to hear the page's console."""

POLL_SECONDS = 0.01
"""How often the test looks whether the listener heard it."""


@pytest.fixture
def with_browser() -> bool:
    """The engine opens a page, in the test's own Browser cache."""
    return True


async def failure(call: Awaitable[object]) -> GoodfetchError:
    """The Goodfetch error `call` raises."""
    with pytest.raises(GoodfetchError) as raised:
        await call
    return raised.value


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


class Throwing:
    """A listener that throws on its first message, and whether it did."""

    def __init__(self) -> None:
        """Has not thrown yet."""
        self.bug = ValueError("the listener's bug")
        self.threw = False

    def __call__(self, _: ConsoleMessage) -> None:
        """Throws the bug."""
        self.threw = True
        raise self.bug


@pytest.mark.covers(
    "story:11-61", "story:11-62", "story:11-63", "error:listener_failed"
)
@pytest.mark.client_case(
    id="listeners-in-order-and-a-thrower-raises-listener-failed", mode="attached"
)
@three_ways
async def test_hears_the_console_in_order_and_raises_a_thrown_listeners_failure(
    way: Way, env: Mapping[str, str], client_case: ClientCaseRecord
) -> None:
    """The next call on the page raises `listener_failed`; the one after it is sent."""
    collie = await way.open(
        lambda: AsyncCollie.connect(env=env), lambda: Collie.connect(env=env)
    )
    run = await collie.start()
    page = await run.new_page()
    texts: list[str] = []
    failing = Throwing()

    heard = page.on("console", lambda message: texts.append(message.text))
    await page.evaluate("() => { console.log('first'); console.log('second'); }")
    await until(lambda: len(texts) >= 2, "both messages")
    heard.close()
    page.on("console", failing)
    await page.evaluate("() => console.log('third')")
    await until(lambda: failing.threw, "the listener's throw")
    refused = await failure(page.evaluate("() => 1"))
    after = await page.evaluate("() => 1 + 1")
    await run.finish()
    await collie.close()

    with client_case.check("the listener heard the console messages in order"):
        assert texts == ["first", "second"]
    raised = (
        "the next call on the page raised listener_failed,"
        " with the listener's exception"
    )
    with client_case.check(raised):
        assert isinstance(refused, ListenerFailedError)
        assert (refused.code, refused.group, refused.details) == (
            "listener_failed",
            "program",
            {"event": "page.console"},
        )
        assert refused.__cause__ is failing.bug
    with client_case.check("the page stayed open and answers the call after it"):
        assert after.value == 2
