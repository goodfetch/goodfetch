"""A sequence read to its end, as an iterator, on an Attached engine (ADR 0081).

The engine is Collie's executable of the same commit, three ways: `async
for` on asyncio and on Trio, and `for` through the sync `Collie` in a
worker thread. The sequence is a Run's Key-value store's keys (ADR 0103),
more than one grant of credits long: read to its end, it gives every key
in order; stopped by leaving the loop, `close()` or, in the async API,
`aclose()`, it gives nothing more, and the Run's next call is answered.
"""

import functools
from collections.abc import Mapping
from typing import TYPE_CHECKING, Literal, cast

import anyio
import pytest
from goodfetch_collie import AsyncCollie, Collie
from goodfetch_test_support import ClientCaseRecord, Way, three_ways

if TYPE_CHECKING:
    from goodfetch_collie import AsyncRun, AsyncSequence, Run, Sequence

PREFIX = "item-"
"""The prefix of the keys the test reads."""

KEYS = [f"{PREFIX}{number:02}" for number in range(1, 21)]
"""The keys the test stores: more than the 16 credits of a first grant."""

Stop = Literal["break", "close", "aclose"]
"""How the program stops reading: `aclose` is the async API's alone."""


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
    """The first key under `PREFIX`, then what reading on gives once `stop` ran.

    A bare loop's `break` releases the iterator, leaving nothing to read on.
    """
    if way is Way.SYNC:
        return await anyio.to_thread.run_sync(
            functools.partial(_sync_read_one_then_stop, sync_run(run), stop)
        )
    read: list[str] = []
    if stop == "break":
        async for key in run.key_value_store.keys(prefix=PREFIX):
            read.append(key)
            break
        return read
    keys: AsyncSequence[str] = run.key_value_store.keys(prefix=PREFIX)
    read.append(await anext(keys))
    if stop == "aclose":
        await keys.aclose()
    else:
        keys.close()
    read.extend([key async for key in keys])
    return read


def _sync_read_one_then_stop(run: "Run", stop: Stop) -> list[str]:
    read: list[str] = []
    if stop == "break":
        for key in run.key_value_store.keys(prefix=PREFIX):
            read.append(key)
            break
        return read
    keys: Sequence[str] = run.key_value_store.keys(prefix=PREFIX)
    read.append(next(keys))
    keys.close()
    read.extend(keys)
    return read


@pytest.mark.covers("story:11-61", "story:11-62", "story:11-63")
@pytest.mark.client_case(
    id="a-sequence-is-an-iterator-that-stops-on-break-and-close", mode="attached"
)
@three_ways
async def test_reads_keys_to_the_end_and_stops_on_break_and_close(
    way: Way, env: Mapping[str, str], client_case: ClientCaseRecord
) -> None:
    """Each stop leaves nothing more to read, and the Run's next read is answered."""
    collie = await way.open(
        lambda: AsyncCollie.connect(env=env), lambda: Collie.connect(env=env)
    )
    run = await collie.start()
    for number, key in enumerate(KEYS):
        await collie.call(
            "key_value_store.set", {"run_id": run.id, "key": key, "value": number}
        )
    await collie.call(
        "key_value_store.set", {"run_id": run.id, "key": "other", "value": 0}
    )

    whole = await read_to_the_end(way, run)
    stops: list[Stop] = (
        ["break", "close"] if way is Way.SYNC else ["break", "close", "aclose"]
    )
    stopped: dict[Stop, tuple[list[str], list[str]]] = {}
    for stop in stops:
        read = await read_one_then_stop(way, run, stop)
        stopped[stop] = (read, await read_to_the_end(way, run))
    await run.finish()
    await collie.close()

    with client_case.check("every key under the prefix was read, in order"):
        assert whole == KEYS
    for stop, (read, after) in stopped.items():
        with client_case.check(f"after {stop}, nothing more was read"):
            assert read == [KEYS[0]]
        with client_case.check(f"after {stop}, the Run's next read was answered"):
            assert after == KEYS
