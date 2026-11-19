"""The end of a crawl handler's attempt passes a program's `except Exception:`.

`AttemptEndedError` derives from `BaseException`, as `asyncio.CancelledError`
does, and still carries its code and group (ADRs 0079 and 0093), so the
SDK raises it out of the handler to what runs the attempt, through a call,
through a sequence's reader and through the sync client's private loop.
TypeScript's `AttemptEndedError` is a `ProgramError`, since TypeScript has
no such split.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

import anyio
import pytest
from goodfetch_collie_client.generated.errors import ERROR_CLASSES as COLLIE_ERRORS
from goodfetch_collie_client.generated.errors import ClosedError
from goodfetch_collie_client.listeners import Order
from goodfetch_collie_client.portal import PrivateLoop
from goodfetch_collie_client.rpc import Channel
from goodfetch_collie_client.sequence import AsyncSequence, Reading
from goodfetch_foxhound.generated.errors import ERROR_CLASSES as FOXHOUND_ERRORS
from goodfetch_foxhound.generated.errors import AttemptEndedError
from goodfetch_test_support.play_script import Finished, play_script

if TYPE_CHECKING:
    from goodfetch_collie_client.goodfetch_error import GoodfetchError
    from goodfetch_collie_client.rpc import TransportEnd

_RUN = "01ke6p4ym0j8x3v5wq2rtbn7hc"


class _Owner:
    """A Connection that hears nothing: the script never ends the channel."""

    async def ended(self, end: TransportEnd) -> GoodfetchError:
        return ClosedError(f"the engine's side ended: {end}")

    def notified(self, method: str, params: object) -> None:
        del method, params  # No notification names no call here.


@pytest.mark.anyio
async def test_a_handlers_except_exception_lets_the_end_of_its_attempt_through() -> (
    None
):
    """The reply raises `AttemptEndedError`, which the handler cannot swallow."""
    played = play_script("sdk-python/crawl/enqueue-after-the-attempt-ended.json")
    channel = Channel(played.transport, _Owner(), {**COLLIE_ERRORS, **FOXHOUND_ERRORS})
    swallowed: list[Exception] = []

    async def handler() -> None:
        try:
            await channel.call(
                "crawl_request.enqueue",
                {
                    "run_id": _RUN,
                    "crawl_request_id": "7",
                    "lease_epoch": 1,
                    "url": "https://shop.example/products/2",
                    "label": "product",
                },
            )
        except Exception as error:
            swallowed.append(error)

    with pytest.raises(AttemptEndedError) as ended:
        await handler()

    assert {
        "swallowed": swallowed,
        "is_exception": isinstance(ended.value, Exception),
        "code": ended.value.code,
        "group": ended.value.group,
        "retryable": ended.value.retryable,
        "operation": ended.value.operation,
        "run_id": ended.value.run_id,
        "ending": played.done(),
    } == {
        "swallowed": [],
        "is_exception": False,
        "code": "attempt_ended",
        "group": "program",
        "retryable": False,
        "operation": "crawl_request.enqueue",
        "run_id": _RUN,
        "ending": Finished(),
    }


def _key(method: str, params: object) -> str | None:
    del method, params  # The stream is refused before any key.
    return None


@pytest.mark.anyio
async def test_a_sequence_on_an_ended_attempt_raises_the_end_to_its_reader() -> None:
    """The sequence's task keeps the end and its reader raises it, as any failure."""
    played = play_script("sdk-python/crawl/keys-after-the-attempt-ended.json")
    channel = Channel(played.transport, _Owner(), {**COLLIE_ERRORS, **FOXHOUND_ERRORS})
    reading: Reading[str] = Reading(
        channel,
        "key_value_store.keys",
        {"run_id": _RUN, "prefix": "item-"},
        _key,
        Order(),
    )
    read: list[str] = []
    raised: list[AttemptEndedError] = []

    async with anyio.create_task_group() as group:
        group.start_soon(reading.run)
        with pytest.raises(AttemptEndedError) as ended:
            async with AsyncSequence(reading) as items:
                read.extend([item async for item in items])
        raised.append(ended.value)

    assert [(read, end.code, end.operation) for end in raised] == [
        ([], "attempt_ended", "key_value_store.keys")
    ]
    assert played.done() == Finished()


def test_a_sync_call_raises_the_end_of_its_attempt_and_its_loop_keeps_serving() -> None:
    """The sync client's private loop hands the end to its caller and lives on."""
    loop = PrivateLoop()

    async def ended() -> str:
        message = "Attempt 1 of crawl request `7` already ended."
        raise AttemptEndedError(message, operation="crawl_request.enqueue")

    async def answered() -> str:
        return "served"

    try:
        with pytest.raises(AttemptEndedError) as raised:
            loop.call(ended)
        after = loop.call(answered)
    finally:
        loop.stop()

    assert (raised.value.code, after) == ("attempt_ended", "served")
