"""A Connection's Schedules (spec 589, ADR 0099): `connection.schedules`.

A sub-object reached without I/O whose calls act on no Schedule, and the
`AsyncSchedule` and `Schedule` handles `create` and `get` answer, whose
members the catalog generates. A Schedule handle holds its ID and has no
`close()`: it names a Schedule its store keeps, so dropping it frees nothing
in the engine. TypeScript's `schedules.ts` is its twin.
"""

from __future__ import annotations

import functools
from dataclasses import dataclass
from typing import TYPE_CHECKING, Generic, NoReturn, Protocol, TypeVar

from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.connection import closed_error
from goodfetch_collie_client.generated.handles import (
    AsyncSchedule as _AsyncScheduleMembers,
)
from goodfetch_collie_client.generated.handles import Schedule as _ScheduleMembers
from goodfetch_collie_client.generated.models import (
    ScheduleCreated,
    ScheduleCreateParams,
    ScheduleGetParams,
    ScheduleId,
    ScheduleInfo,
    ScheduleList,
    ScheduleListParams,
    SchedulePreview,
    SchedulePreviewParams,
)
from goodfetch_collie_client.handle import NotGiven, not_given, timeout_of
from goodfetch_collie_client.markers import warn_experimental
from goodfetch_collie_client.portal import LoopStopped

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Mapping
    from datetime import datetime, timedelta

    from pydantic import BaseModel, JsonValue

    from goodfetch_collie_client.generated.models import (
        ScheduleDefinition,
        ScheduleTrigger,
        ScheduleWarning,
    )
    from goodfetch_collie_client.portal import PrivateLoop

__all__ = [
    "AsyncSchedule",
    "AsyncSchedules",
    "CreatedSchedule",
    "Schedule",
    "Schedules",
]

_Schedule = TypeVar("_Schedule", "AsyncSchedule", "Schedule")
_Result = TypeVar("_Result")

_TIMEOUT_SETTING = "timeout.wait"
"""The setting each Schedule call's own `timeout` sets."""


class SchedulesHost(Protocol):
    """What a Connection's Schedules reach the engine through: its Connection."""

    async def call(
        self, operation: str, params: BaseModel, *, timeout_setting: str | None = None
    ) -> object:
        """Sends one call and returns its result as the engine sent it."""
        ...

    async def raw(self, operation: str, params: Mapping[str, object]) -> JsonValue:
        """Sends the raw call `operation`, its params and result as on the wire."""
        ...

    def ensure_open(self, operation: str) -> None:
        """Raises `closed` once the Connection's close has begun."""
        ...


@dataclass(frozen=True)
class CreatedSchedule(Generic[_Schedule]):
    """What `schedules.create` answers: the Schedule, and what to know of it."""

    schedule: _Schedule
    """The Schedule, under the ID given or made."""
    warnings: tuple[ScheduleWarning, ...]
    """What did not stop its creation but its program should know."""


class AsyncSchedule(_AsyncScheduleMembers):
    """A Schedule a program holds in the async API, by the ID its store keeps it under.

    A program never constructs one: `schedules.create` and `schedules.get`
    answer it.
    """

    @property
    def id(self) -> str:
        """The Schedule's ID, which names it for good. Read without a round trip."""
        return self._host.id.root


class Schedule(_ScheduleMembers):
    """A Schedule a program holds in the sync API, as `AsyncSchedule` is.

    A program never constructs one: `schedules.create` and `schedules.get`
    answer it.
    """

    @property
    def id(self) -> str:
        """The Schedule's ID, which names it for good. Read without a round trip."""
        return self._host.id.root


class AsyncSchedules:
    """A Connection's Schedules in the async API: `connection.schedules`.

    Each call acts on the Schedule store of the engine's major, and none on
    one Schedule: an `AsyncSchedule`'s own members do. A program never
    constructs one.
    """

    def __init__(self, host: SchedulesHost) -> None:
        """Takes the Connection its calls go through."""
        self._host = host

    async def create(
        self,
        definition: ScheduleDefinition,
        *,
        timeout: timedelta | NotGiven | None = not_given,
    ) -> CreatedSchedule[AsyncSchedule]:
        """Records a Schedule, and answers the `AsyncSchedule` that holds it.

        Records a Schedule in the Schedule store, which the Daemon keeps, and
        answers its ID, as given or made (`s` and a lowercase ULID), with
        warnings. Nothing fires yet. A taken ID fails with `schedule_exists`, a
        trigger that does not parse with `schedule_trigger_invalid`. Its time
        zone is the Daemon host's when the definition names none. One store
        transaction: a cancel stops it only before it commits.

        Left out, `timeout` takes `timeout.wait`, by default
        `timedelta(seconds=30)`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_experimental("schedule.create")
        created = await _create(self._host, definition, timeout)
        return CreatedSchedule(
            AsyncSchedule(_AsyncScheduleHost(self._host, created.schedule_id)),
            tuple(created.warnings),
        )

    async def list(
        self, *, timeout: timedelta | NotGiven | None = not_given
    ) -> list[ScheduleInfo]:
        """Lists every Schedule of the Schedule store, by ID.

        Left out, `timeout` takes `timeout.wait`, by default
        `timedelta(seconds=30)`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_experimental("schedule.list")
        return await _list(self._host, timeout)

    async def get(
        self, id: str, *, timeout: timedelta | NotGiven | None = not_given
    ) -> AsyncSchedule:
        """Answers the `AsyncSchedule` of `id` once the store said it holds one.

        Reads the Schedule of an ID. An ID the store does not hold fails with
        `schedule_not_found`. The handle's `info()` reads it again. Left out,
        `timeout` takes `timeout.wait`, by default `timedelta(seconds=30)`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_experimental("schedule.get")
        info = await _get(self._host, id, timeout)
        return AsyncSchedule(_AsyncScheduleHost(self._host, info.schedule_id))

    async def preview(
        self,
        trigger: ScheduleTrigger,
        *,
        time_zone: str | None = None,
        after: datetime | None = None,
        count: int | None = None,
        timeout: timedelta | NotGiven | None = not_given,
    ) -> SchedulePreview:
        """Describes `trigger` in plain words and lists its next Occurrences.

        Describes a Schedule's trigger in plain words and lists its next
        Occurrences in a time zone strictly after `after` (now when absent; 5
        by default, at most 100), including DST transitions. A trigger that
        does not parse fails with `schedule_trigger_invalid`. Left out,
        `timeout` takes `timeout.wait`, by default `timedelta(seconds=30)`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_experimental("schedule.preview")
        return await _preview(self._host, trigger, time_zone, after, count, timeout)


class Schedules:
    """A Connection's Schedules in the sync API, as `AsyncSchedules` are.

    A program never constructs one.
    """

    def __init__(self, loop: PrivateLoop, host: SchedulesHost) -> None:
        """Takes the Connection's private loop and the Connection it calls through."""
        self._loop = loop
        self._host = host

    def create(
        self,
        definition: ScheduleDefinition,
        *,
        timeout: timedelta | NotGiven | None = not_given,
    ) -> CreatedSchedule[Schedule]:
        """Records a Schedule, and answers the `Schedule` that holds it.

        Records a Schedule in the Schedule store, which the Daemon keeps, and
        answers its ID, as given or made (`s` and a lowercase ULID), with
        warnings. Nothing fires yet. A taken ID fails with `schedule_exists`, a
        trigger that does not parse with `schedule_trigger_invalid`. Its time
        zone is the Daemon host's when the definition names none. One store
        transaction: a cancel stops it only before it commits.

        As `AsyncSchedules.create` does.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        operation = "schedule.create"
        warn_experimental(operation)
        created = self._on_loop(
            operation, functools.partial(_create, self._host, definition, timeout)
        )
        return CreatedSchedule(
            self._schedule_of(created.schedule_id), tuple(created.warnings)
        )

    def list(
        self, *, timeout: timedelta | NotGiven | None = not_given
    ) -> list[ScheduleInfo]:
        """Lists every Schedule of the Schedule store, by ID.

        As `AsyncSchedules.list` does.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        operation = "schedule.list"
        warn_experimental(operation)
        return self._on_loop(operation, functools.partial(_list, self._host, timeout))

    def get(
        self, id: str, *, timeout: timedelta | NotGiven | None = not_given
    ) -> Schedule:
        """Answers the `Schedule` of `id`, as `AsyncSchedules.get` does.

        Reads the Schedule of an ID. An ID the store does not hold fails with
        `schedule_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        operation = "schedule.get"
        warn_experimental(operation)
        info = self._on_loop(
            operation, functools.partial(_get, self._host, id, timeout)
        )
        return self._schedule_of(info.schedule_id)

    def preview(
        self,
        trigger: ScheduleTrigger,
        *,
        time_zone: str | None = None,
        after: datetime | None = None,
        count: int | None = None,
        timeout: timedelta | NotGiven | None = not_given,
    ) -> SchedulePreview:
        """Describes `trigger` in plain words and lists its next Occurrences.

        Describes a Schedule's trigger in plain words and lists its next
        Occurrences in a time zone strictly after `after` (now when absent; 5
        by default, at most 100), including DST transitions. A trigger that
        does not parse fails with `schedule_trigger_invalid`.

        As `AsyncSchedules.preview` does.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        operation = "schedule.preview"
        warn_experimental(operation)
        return self._on_loop(
            operation,
            functools.partial(
                _preview, self._host, trigger, time_zone, after, count, timeout
            ),
        )

    def _schedule_of(self, id: ScheduleId) -> Schedule:
        """The handle of the Schedule of ID `id`, whose calls go through the loop."""
        return Schedule(
            _SyncScheduleHost(self._loop, _AsyncScheduleHost(self._host, id))
        )

    def _on_loop(
        self, operation: str, function: Callable[[], Awaitable[_Result]]
    ) -> _Result:
        """Runs `function` on the private loop; `closed` once the loop stopped."""
        try:
            return self._loop.call(function)
        except LoopStopped:
            # Its Connection closed before stopping the loop.
            raise closed_error(operation) from None


async def _create(
    host: SchedulesHost,
    definition: ScheduleDefinition,
    timeout: timedelta | NotGiven | None,
) -> ScheduleCreated:
    """Sends `schedule.create` for `definition`, unless the Connection is closed."""
    params = ScheduleCreateParams.model_construct(definition=definition)
    return result_of(
        ScheduleCreated, await _sent(host, "schedule.create", params, timeout)
    )


async def _list(
    host: SchedulesHost, timeout: timedelta | NotGiven | None
) -> list[ScheduleInfo]:
    """Sends `schedule.list`, unless the Connection is closed."""
    params = ScheduleListParams.model_construct()
    listed = await _sent(host, "schedule.list", params, timeout)
    return result_of(ScheduleList, listed).schedules


async def _get(
    host: SchedulesHost, id: str, timeout: timedelta | NotGiven | None
) -> ScheduleInfo:
    """Sends `schedule.get` for `id`, unless the Connection is closed.

    The ID is sent as given: the engine checks it, as it checks every
    params (STYLE.md).
    """
    params = ScheduleGetParams.model_construct(id=ScheduleId.model_construct(id))
    return result_of(ScheduleInfo, await _sent(host, "schedule.get", params, timeout))


async def _preview(
    host: SchedulesHost,
    trigger: ScheduleTrigger,
    time_zone: str | None,
    after: datetime | None,
    count: int | None,
    timeout: timedelta | NotGiven | None,
) -> SchedulePreview:
    """Sends `schedule.preview`, unless the Connection is closed."""
    params = SchedulePreviewParams.model_construct(trigger=trigger)
    if time_zone is not None:
        params.time_zone = time_zone
    if after is not None:
        params.after = after
    if count is not None:
        params.count = count
    return result_of(
        SchedulePreview, await _sent(host, "schedule.preview", params, timeout)
    )


async def _sent(
    host: SchedulesHost,
    operation: str,
    params: ScheduleCreateParams
    | ScheduleListParams
    | ScheduleGetParams
    | SchedulePreviewParams,
    timeout: timedelta | NotGiven | None,
) -> object:
    """Sends `operation` with its own `timeout`, unless the Connection is closed."""
    host.ensure_open(operation)
    if not isinstance(timeout, NotGiven):
        params.timeout = timeout_of(timeout, _TIMEOUT_SETTING)
    return await host.call(operation, params, timeout_setting=_TIMEOUT_SETTING)


def _nothing_held(what: str) -> NoReturn:
    """A Schedule handle holds no value, close or listener."""
    message = f"invariant: a Schedule {what}"
    raise TypeError(message)


class _AsyncScheduleHost:
    """What an `AsyncSchedule` reaches its engine through: its Connection.

    It opens, closes and listens to nothing.
    """

    def __init__(self, host: SchedulesHost, id: ScheduleId) -> None:
        self._host = host
        self._id = id

    @property
    def id(self) -> ScheduleId:
        """The ID each of the handle's calls carries."""
        return self._id

    async def call(
        self, operation: str, params: BaseModel, *, timeout_setting: str | None = None
    ) -> object:
        """Sends one call, unless the Connection is closed."""
        self._host.ensure_open(operation)
        return await self._host.call(operation, params, timeout_setting=timeout_setting)

    async def raw(self, operation: str, params: Mapping[str, object]) -> JsonValue:
        """Sends the raw call `operation`, unless the Connection is closed."""
        self._host.ensure_open(operation)
        return await self._host.raw(operation, params)

    def child(self, id: object) -> NoReturn:
        """A Schedule opens no value."""
        del id
        _nothing_held("opens no value")

    @property
    def connection(self) -> NoReturn:
        """A Schedule carries no copy connection."""
        _nothing_held("carries no copy connection")

    async def close(self, operation: str, params: BaseModel) -> NoReturn:
        """A Schedule has no `close()`."""
        del operation, params
        _nothing_held("has no close()")

    def listen(
        self,
        operation: str,
        params: BaseModel,
        event: str,
        payload: object,
        listener: object,
    ) -> NoReturn:
        """A Schedule has no listener."""
        del operation, params, event, payload, listener
        _nothing_held("has no listener")

    def listen_published(
        self,
        operation: str,
        params: BaseModel,
        event: str,
        param: str,
        payload: object,
        listener: object,
    ) -> NoReturn:
        """A Schedule has no listener."""
        del operation, params, event, param, payload, listener
        _nothing_held("has no listener")


class _SyncScheduleHost:
    """What a sync `Schedule` reaches its engine through: its async twin's host."""

    def __init__(self, loop: PrivateLoop, host: _AsyncScheduleHost) -> None:
        self._loop = loop
        self._host = host

    @property
    def id(self) -> ScheduleId:
        """The ID each of the handle's calls carries."""
        return self._host.id

    @property
    def connection(self) -> NoReturn:
        """A Schedule carries no copy connection."""
        _nothing_held("carries no copy connection")

    @property
    def loop(self) -> NoReturn:
        """A Schedule carries no copy loop."""
        _nothing_held("carries no copy loop")

    def call(
        self, operation: str, params: BaseModel, *, timeout_setting: str | None = None
    ) -> object:
        """Sends one call on the private loop and waits for its result."""
        try:
            return self._loop.call(
                functools.partial(
                    self._host.call, operation, params, timeout_setting=timeout_setting
                )
            )
        except LoopStopped:
            # Its Connection closed before stopping the loop.
            raise closed_error(operation) from None

    def raw(self, operation: str, params: Mapping[str, object]) -> JsonValue:
        """Sends the raw call on the private loop and waits for its result."""
        try:
            return self._loop.call(functools.partial(self._host.raw, operation, params))
        except LoopStopped:
            # Its Connection closed before stopping the loop.
            raise closed_error(operation) from None

    def child(self, id: object) -> NoReturn:
        """A Schedule opens no value."""
        del id
        _nothing_held("opens no value")

    def close(self, operation: str, params: BaseModel) -> NoReturn:
        """A Schedule has no `close()`."""
        del operation, params
        _nothing_held("has no close()")

    def listen(
        self,
        operation: str,
        params: BaseModel,
        event: str,
        payload: object,
        listener: object,
    ) -> NoReturn:
        """A Schedule has no listener."""
        del operation, params, event, payload, listener
        _nothing_held("has no listener")

    def listen_published(
        self,
        operation: str,
        params: BaseModel,
        event: str,
        param: str,
        payload: object,
        listener: object,
    ) -> NoReturn:
        """A Schedule has no listener."""
        del operation, params, event, param, payload, listener
        _nothing_held("has no listener")
