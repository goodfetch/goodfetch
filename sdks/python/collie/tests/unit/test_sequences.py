"""A sequence read to its end, as an iterator, on the Scripted engine (ADR 0081).

Each test plays a script of the shared corpus over the in-memory transport,
which fails the test on a message it does not expect and on a step left
unused, three ways: `async for` on asyncio and on Trio, and `for` through
the sync `Collie`. The sequence is a Run's Key-value store's keys (ADR
0103): `keys()` sends its call when called, grants credits as the program
reads, and stops it with `$/cancel`, sent before the Run's next call, when
the program breaks out of its loop, closes the iterator or releases it.
TypeScript's `sequence.test.ts` is its twin.
"""

import functools
import gc
from typing import TYPE_CHECKING, Literal, cast

import anyio
import pytest
from goodfetch_collie import AsyncCollie
from goodfetch_collie.testing import async_connect_over, connect_over
from goodfetch_test_support import Finished, PlayedScript, Way, play_script, three_ways

if TYPE_CHECKING:
    from goodfetch_collie import AsyncRun, AsyncSequence, Run, Sequence

PREFIX = "item-"
"""The prefix the scripts' keys start with."""

Stop = Literal["break", "close", "aclose", "with", "release"]
"""How the program stops reading: `aclose` is the async API's alone."""


async def collie_on(way: Way, script: str) -> tuple[AsyncCollie, PlayedScript]:
    """The Scripted engine playing `script`, and a Connection this way opened on it."""
    played = play_script(f"sdk-python/{script}")
    collie = await way.open(
        lambda: async_connect_over(played.transport),
        lambda: connect_over(played.transport),
    )
    return collie, played


def sync_run(run: "AsyncRun") -> "Run":
    """The sync `Run` the sync way's proxy hands in place of `run`."""
    return cast("Run", run)


async def read_to_the_end(way: Way, run: "AsyncRun") -> list[str]:
    """Every key under `PREFIX`, read this way's: `async for`, or `for` in a thread."""
    if way is Way.SYNC:
        store = sync_run(run).key_value_store
        return await anyio.to_thread.run_sync(lambda: list(store.keys(prefix=PREFIX)))
    return [key async for key in run.key_value_store.keys(prefix=PREFIX)]


async def read_one_then_stop(way: Way, run: "AsyncRun", stop: Stop) -> list[str]:
    """The first key under `PREFIX`, then what reading on gives once `stop` stopped it.

    `break` and `release` leave no iterator to read on, so give nothing more.
    """
    if way is Way.SYNC:
        return await anyio.to_thread.run_sync(
            functools.partial(_sync_read_one_then_stop, sync_run(run), stop)
        )
    store = run.key_value_store
    read: list[str] = []
    if stop == "break":
        async for key in store.keys(prefix=PREFIX):
            read.append(key)
            break
        return read
    if stop == "release":
        read.append(await anext(store.keys(prefix=PREFIX)))
        gc.collect()
        return read
    keys: AsyncSequence[str] = store.keys(prefix=PREFIX)
    if stop == "with":
        async with keys:
            read.append(await anext(keys))
    else:
        read.append(await anext(keys))
        if stop == "aclose":
            await keys.aclose()
        else:
            keys.close()
    read.extend([key async for key in keys])
    return read


def _sync_read_one_then_stop(run: "Run", stop: Stop) -> list[str]:
    store = run.key_value_store
    read: list[str] = []
    if stop == "break":
        for key in store.keys(prefix=PREFIX):
            read.append(key)
            break
        return read
    if stop == "release":
        read.append(next(store.keys(prefix=PREFIX)))
        gc.collect()
        return read
    keys: Sequence[str] = store.keys(prefix=PREFIX)
    if stop == "with":
        with keys:
            read.append(next(keys))
    else:
        read.append(next(keys))
        keys.close()
    read.extend(keys)
    return read


@pytest.mark.covers("story:11-61", "story:11-63")
@three_ways
async def test_reads_its_keys_to_the_end_granting_credits_as_it_reads(
    way: Way,
) -> None:
    """The call goes when `keys()` is called; 8 credits go back once 8 are read."""
    collie, played = await collie_on(way, "run/keys-read.json")
    run = await collie.start()

    keys = await read_to_the_end(way, run)
    await run.finish()
    await collie.close()

    assert keys == [f"item-{number}" for number in range(1, 9)]
    assert played.done() == Finished()


@pytest.mark.covers("story:11-61", "story:11-62")
@pytest.mark.parametrize("stop", ["break", "close", "with", "release"])
@three_ways
async def test_stopping_reading_cancels_the_call_before_the_runs_next_call(
    way: Way, stop: Stop
) -> None:
    """`$/cancel` reaches the engine before `run.finish`; nothing more is read."""
    await stops_the_call(way, stop)


@pytest.mark.covers("story:11-62")
@pytest.mark.anyio
async def test_aclose_cancels_the_call_before_the_runs_next_call() -> None:
    """The async API's `aclose()` stops reading as `close()` does."""
    await stops_the_call(Way.ASYNCIO, "aclose")


async def stops_the_call(way: Way, stop: Stop) -> None:
    """Reads one key, stops by `stop`, and checks the call was cancelled in order."""
    collie, played = await collie_on(way, "run/keys-stopped.json")
    run = await collie.start()

    read = await read_one_then_stop(way, run, stop)
    await run.finish()
    await collie.close()

    assert read == ["item-1"]
    assert played.done() == Finished()
