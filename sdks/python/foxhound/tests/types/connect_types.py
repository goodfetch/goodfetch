"""What `connect()` takes (ADRs 0083 and 0104), checked by pyright and mypy.

`settings` takes Collie's and Foxhound's keys, never one the engine reads
only from its configuration (P38). `fetch_backends` and `sinks` take the
program's own handlers by name: coroutine functions on `AsyncFoxhound`,
plain functions on the sync `Foxhound`, each of its kind's shape. mypy's
`--strict` reports an ignore that no longer silences an error.
TypeScript's `settings.test-d.ts` of Foxhound's package is its twin.
"""

from typing import TYPE_CHECKING

from goodfetch_foxhound import (
    AsyncFoxhound,
    BackendRequest,
    BackendResponse,
    Foxhound,
    SinkDelivery,
)

if TYPE_CHECKING:
    from goodfetch_foxhound.generated.settings import FoxhoundSettings


def _backend(request: BackendRequest) -> BackendResponse:
    return BackendResponse(status=200, headers={"x-url": request.url}, body=b"")


async def _async_backend(request: BackendRequest) -> BackendResponse:
    return _backend(request)


def _sink(delivery: SinkDelivery) -> None:
    del delivery


async def _async_sink(delivery: SinkDelivery) -> None:
    del delivery


async def registers_each_ways_handlers() -> None:
    """Coroutine functions on `AsyncFoxhound`, plain ones on `Foxhound`."""
    await AsyncFoxhound.connect(
        fetch_backends={"mine": _async_backend},
        sinks={"mine": _async_sink},
        settings={"crawl": {"max_depth": 2}},
    )
    Foxhound.connect(fetch_backends={"mine": _backend}, sinks={"mine": _sink})


async def refuses_the_other_ways_handlers() -> None:
    """A plain function on `AsyncFoxhound`, or a coroutine one on `Foxhound`."""
    await AsyncFoxhound.connect(fetch_backends={"mine": _backend})  # type: ignore[dict-item]
    Foxhound.connect(sinks={"mine": _async_sink})  # type: ignore[dict-item]


def refuses_a_configuration_only_key() -> None:
    """The engine reads `storage.*`, `log.*` and `daemon.*` from configuration alone."""
    a: FoxhoundSettings = {"storage": {"low_free": 1_000}}  # type: ignore[typeddict-unknown-key]
    b: FoxhoundSettings = {"log": {"level": "debug"}}  # type: ignore[typeddict-unknown-key]
    c: FoxhoundSettings = {"daemon": {"address": "127.0.0.1:7341"}}  # type: ignore[typeddict-unknown-key]
    del a, b, c
