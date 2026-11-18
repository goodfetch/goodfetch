"""A Way hands a case the API it calls (ADR 0069)."""

import threading
from dataclasses import dataclass

import pytest
from goodfetch_test_support import Way


@dataclass(frozen=True)
class AsyncRun:
    """An async API's Run."""

    id: str

    async def finish(self) -> str:
        """Says which API finished it."""
        return f"async {self.id}"

    async def close(self) -> None:
        """Closes nothing."""


@dataclass(frozen=True)
class SyncRun:
    """A sync API's Run, which records the thread it was closed on in `threads`."""

    id: str
    threads: list[threading.Thread]

    def finish(self) -> str:
        """Says which API finished it."""
        return f"sync {self.id}"

    def close(self) -> None:
        """Records the thread it runs on."""
        self.threads.append(threading.current_thread())


@dataclass(frozen=True)
class AsyncConnection:
    """An async API's Connection."""

    async def start(self, run_id: str) -> AsyncRun:
        """Starts an async Run."""
        return AsyncRun(run_id)


@dataclass(frozen=True)
class SyncConnection:
    """A sync API's Connection, which records the thread it opens on in `threads`."""

    threads: list[threading.Thread]

    def __post_init__(self) -> None:
        """Records the thread it is opened on."""
        self.threads.append(threading.current_thread())

    def start(self, run_id: str) -> SyncRun:
        """Starts a sync Run."""
        return SyncRun(run_id, self.threads)


@pytest.mark.anyio
async def test_the_sync_way_hands_a_value_as_it_is_and_awaits_each_method() -> None:
    """A property is read as the sync API holds it; a method runs off the loop."""
    run = Way.SYNC.api(AsyncRun("first"), SyncRun("second", []))

    assert (run.id, await run.finish()) == ("second", "sync second")


async def open_async() -> AsyncConnection:
    """Opens the async API's Connection."""
    return AsyncConnection()


@pytest.mark.anyio
async def test_the_sync_way_opens_off_the_loop_and_so_hands_what_it_opens() -> None:
    """A value with `close`, such as a Run, is awaited off the loop as its opener is."""
    threads: list[threading.Thread] = []
    connection = await Way.SYNC.open(open_async, lambda: SyncConnection(threads))
    run = await connection.start("nightly")

    await run.close()

    assert (run.id, await run.finish()) == ("nightly", "sync nightly")
    assert len(threads) == 2
    assert threading.current_thread() not in threads


@pytest.mark.anyio
async def test_an_async_way_opens_the_async_api() -> None:
    """The async ways await the async opening on the test's own loop."""
    connection = await Way.ASYNCIO.open(open_async, lambda: SyncConnection([]))

    assert isinstance(connection, AsyncConnection)
