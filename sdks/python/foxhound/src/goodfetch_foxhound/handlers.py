"""The program's own Plugins of Foxhound's kinds (ADRs 0033 and 0104).

`connect()` registers them by name, without a process Plugin: a Fetch
backend a setting such as `fetch.backend` chooses, under `fetch_backends`,
and a Sink that serves the `sinks.<name>` entry of its name, under `sinks`.
`AsyncFoxhound` takes coroutine functions and the sync `Foxhound` plain
functions (ADR 0081). Each runs with the program's full rights, never
confined. The engine's calls to them are each kind's own work.
TypeScript's `handlers.ts` is its twin.
"""

from __future__ import annotations

from collections.abc import Awaitable, Callable, Mapping, Sequence
from dataclasses import dataclass
from typing import TYPE_CHECKING, TypeAlias

if TYPE_CHECKING:
    from goodfetch_foxhound.generated.models import DatasetRecord

__all__ = [
    "AsyncFetchBackendHandler",
    "AsyncSinkHandler",
    "BackendRequest",
    "BackendResponse",
    "FetchBackendHandler",
    "SinkDelivery",
    "SinkHandler",
]


@dataclass(frozen=True)
class BackendRequest:
    """A request the engine hands the program's Fetch backend.

    It carries no credential: the engine sends a request that needs one
    itself (ADRs 0092 and 0104).
    """

    url: str
    """The URL to fetch."""
    method: str
    """Its method, such as `GET`."""
    headers: Mapping[str, str]
    """Its headers, by name."""
    body: bytes | None
    """Its body, or `None` for none."""
    proxy: str | None
    """The proxy to send it through, with no password, or `None` for none."""


@dataclass(frozen=True)
class BackendResponse:
    """What the program's Fetch backend answers a request with."""

    status: int
    """Its HTTP status."""
    headers: Mapping[str, str]
    """Its headers, by name."""
    body: bytes
    """Its body."""


@dataclass(frozen=True)
class SinkDelivery:
    """A delivery of committed Records to the program's own Sink.

    Its delivery ID and Run epoch make it safe to retry (ADR 0008).
    """

    delivery_id: str
    """Its ID, the same on each retry of it."""
    run_epoch: int
    """The Run's epoch it was made in."""
    dataset: str
    """The Dataset's name."""
    records: Sequence[DatasetRecord]
    """The Records it delivers, in commit order."""


AsyncFetchBackendHandler: TypeAlias = Callable[
    [BackendRequest], Awaitable[BackendResponse]
]
"""A Fetch backend of the program's own, which `AsyncFoxhound.connect()` registers."""

FetchBackendHandler: TypeAlias = Callable[[BackendRequest], BackendResponse]
"""A Fetch backend of the program's own, which `Foxhound.connect()` registers."""

AsyncSinkHandler: TypeAlias = Callable[[SinkDelivery], Awaitable[None]]
"""A Sink of the program's own, which `AsyncFoxhound.connect()` registers."""

SinkHandler: TypeAlias = Callable[[SinkDelivery], None]
"""A Sink of the program's own, which the sync `Foxhound.connect()` registers."""
