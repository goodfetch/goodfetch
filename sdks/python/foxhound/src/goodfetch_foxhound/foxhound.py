"""Foxhound's Connection types (ADRs 0078, 0081, 0090 and 0095).

`AsyncFoxhound.connect()` and the sync `Foxhound.connect()` find Foxhound's
executable, start it as an Attached engine with exactly the program's
environment map, finish `hello`, naming Foxhound, and set the Connection's
Limits before they return, so a Collie engine refuses it there with
`not_in_product`. Everything else is the Collie client package's common
Connection, whose members `AsyncCollie` and `Collie` carry under the same
names; its Runs, results and error classes are that package's own, so a
helper written for Collie takes them (ADR 0095). Foxhound contains Collie
(ADR 0084), so its Connection raises the classes of Collie's Error codes and
of its own. Collie's `collie.py` and TypeScript's `foxhound.ts` are its
twins.
"""

from __future__ import annotations

import functools
import threading
from pathlib import Path
from typing import TYPE_CHECKING

from goodfetch_collie_client.connection import ClientConnection, closed_error
from goodfetch_collie_client.connection_type import AsyncConnection, Connection
from goodfetch_collie_client.generated.errors import (
    ERROR_CLASSES as COLLIE_ERROR_CLASSES,
)
from goodfetch_collie_client.generated.models import Product
from goodfetch_collie_client.generated.operations import (
    DEPRECATED_OPERATIONS,
    EXPERIMENTAL_OPERATIONS,
)
from goodfetch_collie_client.generated.operations import (
    OPERATION_CLASSES as COLLIE_OPERATION_CLASSES,
)
from goodfetch_collie_client.generated.settings import DEPRECATED_SETTINGS
from goodfetch_collie_client.handle import NotGiven, not_given
from goodfetch_collie_client.listeners import MAX_LISTENER_QUEUE
from goodfetch_collie_client.markers import warn_deprecated_settings, warn_entry
from goodfetch_collie_client.portal import LoopStopped, PrivateLoop
from goodfetch_collie_client.product_connection import (
    ProductPackage,
    open_engine,
    open_over,
)
from goodfetch_collie_client.raw_call import raw_params
from goodfetch_collie_client.run import AsyncRun, Run
from goodfetch_collie_client.run_view import RunView

from goodfetch_foxhound.crawl_run import CrawlRuns
from goodfetch_foxhound.generated.connection import (
    FOXHOUND_ENGINE_PATH_VARIABLE,
    FOXHOUND_EXECUTABLE,
)
from goodfetch_foxhound.generated.errors import (
    ERROR_CLASSES as FOXHOUND_ERROR_CLASSES,
)
from goodfetch_foxhound.generated.operations import (
    DEPRECATED_OPERATIONS as FOXHOUND_DEPRECATED_OPERATIONS,
)
from goodfetch_foxhound.generated.operations import (
    EXPERIMENTAL_OPERATIONS as FOXHOUND_EXPERIMENTAL_OPERATIONS,
)
from goodfetch_foxhound.generated.operations import (
    OPERATION_CLASSES as FOXHOUND_OPERATION_CLASSES,
)
from goodfetch_foxhound.generated.settings import (
    DEPRECATED_SETTINGS as FOXHOUND_DEPRECATED_SETTINGS,
)

if TYPE_CHECKING:
    import os
    from collections.abc import Awaitable, Callable, Generator, Iterable, Mapping
    from datetime import timedelta

    from anyio.abc import ByteStream
    from goodfetch_collie_client.connection import EngineInfo
    from goodfetch_collie_client.engine_choice import EngineChoice
    from goodfetch_collie_client.generated.models import (
        Recording,
        SettingsExplanation,
    )
    from goodfetch_collie_client.generated.settings import RunSettings
    from goodfetch_collie_client.handlerless import CatalogTarget
    from goodfetch_collie_client.limits import Limits
    from goodfetch_collie_client.run_view import AsyncRunView
    from pydantic import JsonValue

    from goodfetch_foxhound.crawl import AsyncCrawl, Crawl
    from goodfetch_foxhound.crawl_run import AsyncCrawlRun, CrawlRun
    from goodfetch_foxhound.generated.models import RequestSeed, SitemapSeed
    from goodfetch_foxhound.generated.settings import CrawlSettings, FoxhoundSettings

__all__ = [
    "AsyncFoxhound",
    "Connecting",
    "Foxhound",
    "async_connect_over",
    "connect_over",
]

_FOXHOUND = ProductPackage(
    needs=Product.foxhound,
    name="Foxhound",
    package="goodfetch-foxhound",
    executable=FOXHOUND_EXECUTABLE,
    engine_path_variable=FOXHOUND_ENGINE_PATH_VARIABLE,
    error_classes={**COLLIE_ERROR_CLASSES, **FOXHOUND_ERROR_CLASSES},
    bin_folder=Path(__file__).parent / "bin",
    operation_classes={**COLLIE_OPERATION_CLASSES, **FOXHOUND_OPERATION_CLASSES},
)
"""What makes this package Foxhound's.

The Product its `hello` names, its executable and the one variable it reads:
Collie's variable is another Product's and is never read here.
"""


class AsyncFoxhound(AsyncConnection):
    """A Connection to a Foxhound engine: the entry point of the async API.

    Each one is independent, and many may be open in one process. Opened by
    `connect()`, never constructed directly; `async with` closes it. A
    helper typed for `AsyncConnection` takes it as it takes an `AsyncCollie`.
    """

    def __init__(self, connection: ClientConnection) -> None:
        """Wraps an open Connection, and serves its crawls' handlers."""
        self._connection = connection
        self._crawls = CrawlRuns(connection)

    @staticmethod
    def connect(
        *,
        env: Mapping[str, str] | None = None,
        engine: EngineChoice = "auto",
        limits: Limits | None = None,
        timeout: timedelta | NotGiven | None = not_given,
        max_listener_queue: int = MAX_LISTENER_QUEUE,
        settings: FoxhoundSettings | None = None,
        project: str | os.PathLike[str] | NotGiven | None = not_given,
    ) -> Connecting:
        """Reaches Foxhound's engine and finishes `hello`, naming Foxhound.

        Await it, or open it with `async with`, which closes it. It takes
        `AsyncCollie.connect()`'s arguments and raises as it does, reading
        `GOODFETCH_FOXHOUND_ENGINE_PATH` where Collie reads its own variable
        and starting the executable `goodfetch-foxhound`'s platform wheel
        holds. A Collie engine, which lacks Foxhound, refuses `hello` with
        `not_in_product`, raised here as `NotInProductError` before any
        other call: its details name the Product needed, the engine's
        Product and version and the install command of the channel this
        Client came from, which its `fix` names. `settings` are the
        Connection's own, Collie's and Foxhound's, sent in `hello`, which
        every Run it starts inherits (ADR 0083), and `project` the project
        file read, as `AsyncCollie.connect()` reads it. Importing the
        package or connecting installs no signal handler (ADR 0078).
        """
        return Connecting(
            functools.partial(
                open_engine,
                _FOXHOUND,
                env=env,
                engine=engine,
                limits=limits,
                connect_timeout=timeout,
                max_listener_queue=max_listener_queue,
                settings=settings,
                project=project,
            )
        )

    @property
    def engine(self) -> EngineInfo:
        """The engine reached, read without a round trip, as `AsyncCollie.engine`."""
        return self._connection.engine

    async def start(
        self,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
        settings: RunSettings | None = None,
    ) -> AsyncRun:
        """Leaving the scope or `close()` stops the Run with a Checkpoint.

        `finish()` ends it. It starts the Run as `AsyncCollie.start` does.

        In a service, close the Run on `SIGINT` and `SIGTERM`: the SDK installs
        no signal handler, and a signal the program does not handle ends it as
        a crash would (ADRs 0078, 0093). Python's `KeyboardInterrupt` unwinds
        into `close()`; `SIGTERM` does not.

        Starts a Run its program drives, and answers once it is started. A Run ID
        names the Run for good, and a start under one acts on what the Run index
        holds (ADR 0102): no such Run begins it, `new`; an interrupted Run, stopped,
        failed, or running with no live writer, resumes from its committed state at
        the next run epoch, `resumed`, as `run.resume` does; a finished Run is
        answered as it ended with no handler called, `ended`. A Run live with a
        writer, in this engine or another, fails with `run_in_use`; a Run started
        with other inputs or by another caller with `run_conflict` listing each
        difference; a Run the Run index lists whose state is missing or unreadable
        with `run_state_unavailable`, never starting afresh.

        Experimental: outside semver until it is promoted (ADR 0030).

        The Run is the Collie client package's own `AsyncRun` (ADR 0095).
        """
        warn_entry("run.start", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        warn_deprecated_settings(settings, DEPRECATED_SETTINGS)
        return await self._connection.start(
            run_id=run_id, tags=tags, limits=limits, settings=settings
        )

    async def start_recipe(
        self,
        recipe: str,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
        settings: RunSettings | None = None,
        recording: Recording | None = None,
        timeout: timedelta | NotGiven | None = not_given,
        detached: bool = False,
    ) -> AsyncRun:
        """Starts a Recipe's run, as `AsyncCollie.start_recipe`.

        Starts a Recipe's run, a Run with no handler in the program, and answers once
        it is started. Its Run ID works as `run.start`'s. With `detached`, the Run
        outlives the program that started it: closing its value frees only the
        value, `run.finish` waits for its end and `run.stop` stops it.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("recipe.start", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        warn_deprecated_settings(settings, DEPRECATED_SETTINGS)
        return await self._connection.start_recipe(
            recipe,
            run_id=run_id,
            tags=tags,
            limits=limits,
            settings=settings,
            recording=recording,
            timeout=timeout,
            detached=detached,
        )

    async def start_catalog_run(
        self,
        target: CatalogTarget,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
        settings: RunSettings | None = None,
        recording: Recording | None = None,
        timeout: timedelta | NotGiven | None = not_given,
        detached: bool = False,
    ) -> AsyncRun:
        """Starts a code-free catalog run, as `AsyncCollie.start_catalog_run`.

        Starts a code-free catalog run, a Run with no handler in the program whose
        verbs come from the catalog (ADR 0036): an Extraction of a page by a schema,
        `extract`, an Agent task, `agent`, or a Plan replayed with no model call,
        `plan_replay`; and answers once it is started. Its Run ID works as
        `run.start`'s. With `detached`, the Run outlives the program that started
        it: closing its value frees only the value, `run.finish` waits for its end
        and `run.stop` stops it.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("catalog_run.start", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        warn_deprecated_settings(settings, DEPRECATED_SETTINGS)
        return await self._connection.start_catalog_run(
            target,
            run_id=run_id,
            tags=tags,
            limits=limits,
            settings=settings,
            recording=recording,
            timeout=timeout,
            detached=detached,
        )

    async def start_crawl(
        self,
        crawl: AsyncCrawl,
        *,
        seeds: Iterable[RequestSeed | SitemapSeed],
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        settings: CrawlSettings | None = None,
    ) -> AsyncCrawlRun:
        """Starts `crawl` from `seeds`, at least one, and serves its handlers.

        Starts a crawl from its seeds, and answers once its Run is started.
        The engine then leases each work item to the program's handler as a
        `crawl_request.handle` request, and the crawl's Run finishes once
        every work item is done, or, with `crawl.keep_alive`, once
        `run.finish` lets it and every work item is done. A Run ID works as
        `run.start`'s.

        Each work item goes to the handler of its Label, as a task beside
        this Connection's (ADR 0093). A seed is a `seed` request, or a
        `sitemap` seed of a sitemap's entries. Before anything is sent, a
        Label without a handler fails with `label_unhandled`, and a seed, or
        a sitemap seed's rule, whose Label's name `crawl` does not declare
        with `label_unknown`, since a seed carries its Label's name only.
        `settings` are the Run's own,
        Collie's and Foxhound's, such as
        `{"crawl": {"handler": {"concurrency": 4}}}` (ADRs 0083 and 0093).

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry(
            "crawl.start",
            FOXHOUND_EXPERIMENTAL_OPERATIONS,
            FOXHOUND_DEPRECATED_OPERATIONS,
        )
        warn_deprecated_settings(settings, FOXHOUND_DEPRECATED_SETTINGS)
        return await self._crawls.start(
            crawl, seeds=seeds, run_id=run_id, tags=tags, settings=settings
        )

    async def open_run(self, run_id: str) -> AsyncRunView:
        """Opens a view of a Run by its ID, as `AsyncCollie.open_run`."""
        return await self._connection.open_run(run_id)

    async def call(
        self,
        operation: str,
        params: Mapping[str, JsonValue] | None = None,
        *,
        timeout: timedelta | NotGiven | None = not_given,
    ) -> JsonValue:
        """Sends `operation`, one acting on no value, as the wire's JSON (ADR 0099).

        As `AsyncCollie.call`: the way down to an operation the SDK does
        not generate, such as `run.list`.
        """
        sent = raw_params(operation, params, timeout)
        return await self._connection.raw(operation, sent)

    async def explain_settings(
        self, keys: Iterable[str] | None = None
    ) -> SettingsExplanation:
        """What the Connection runs with, as `AsyncCollie.explain_settings` says.

        Shows each setting's effective value, the layer that set it and the layer
        that locked it, from the built-in defaults, the Operator policy, user
        configuration, the project configuration given and the run layer: the
        Connection's own settings with the run values given over them, or a live
        Run's as it opened when `run_id` names one; and lists the keys only the
        other Product honours as that Product's.
        """
        return await self._connection.explain_settings(keys)

    async def close(self) -> None:
        """Closes the Connection, as `AsyncCollie.close`; idempotent."""
        await self._connection.close()


class Connecting:
    """An `AsyncFoxhound` being opened: await it, or open it with `async with`."""

    def __init__(self, opening: Callable[[], Awaitable[ClientConnection]]) -> None:
        """Takes what opens the Connection, run once it is awaited or entered."""
        self._opening = opening
        self._foxhound: AsyncFoxhound | None = None

    def __await__(self) -> Generator[object, None, AsyncFoxhound]:
        """Opens the Connection."""
        return self._open().__await__()

    async def __aenter__(self) -> AsyncFoxhound:
        """Opens the Connection for an `async with` block."""
        self._foxhound = await self._open()
        return self._foxhound

    async def __aexit__(self, *_: object) -> None:
        """Closes the Connection the block opened."""
        if self._foxhound is not None:
            await self._foxhound.close()

    async def _open(self) -> AsyncFoxhound:
        return AsyncFoxhound(await self._opening())


class Foxhound(Connection):
    """A Connection to a Foxhound engine: the entry point of the sync API.

    It runs on a private daemon thread of its own, never on the caller's
    event loop, as the sync `Collie` does, and any number of threads may
    share it and its Runs (ADR 0081). Opened by `connect()`, never
    constructed directly; `with` closes it. A helper typed for `Connection`
    takes it as it takes a `Collie`.
    """

    def __init__(self, loop: PrivateLoop, connection: ClientConnection) -> None:
        """Takes the private loop and the open Connection it drives there."""
        self._loop = loop
        self._connection = connection
        self._crawls = CrawlRuns(connection)
        self._close_lock = threading.Lock()
        self._closed = False
        self._close_failure: Exception | None = None

    @staticmethod
    def connect(
        *,
        env: Mapping[str, str] | None = None,
        engine: EngineChoice = "auto",
        limits: Limits | None = None,
        timeout: timedelta | NotGiven | None = not_given,
        max_listener_queue: int = MAX_LISTENER_QUEUE,
        settings: FoxhoundSettings | None = None,
        project: str | os.PathLike[str] | NotGiven | None = not_given,
    ) -> Foxhound:
        """Reaches Foxhound's engine and finishes `hello`, naming Foxhound.

        Open it with `with`, which closes it. It takes and raises what
        `AsyncFoxhound.connect()` does, `not_in_product` from a Collie
        engine included, leaving neither Attached engine nor thread behind.
        """
        return _open_sync(
            functools.partial(
                open_engine,
                _FOXHOUND,
                env=env,
                engine=engine,
                limits=limits,
                connect_timeout=timeout,
                max_listener_queue=max_listener_queue,
                settings=settings,
                project=project,
            )
        )

    @property
    def engine(self) -> EngineInfo:
        """The engine reached, read without a round trip, as `Collie.engine`."""
        return self._connection.engine

    def start(
        self,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
        settings: RunSettings | None = None,
    ) -> Run:
        """Leaving the scope or `close()` stops the Run with a Checkpoint.

        `finish()` ends it. It starts the Run as `Collie.start` does.

        In a service, close the Run on `SIGINT` and `SIGTERM`: the SDK installs
        no signal handler, and a signal the program does not handle ends it as
        a crash would (ADRs 0078, 0093). Python's `KeyboardInterrupt` unwinds
        into `close()`; `SIGTERM` does not.

        Starts a Run its program drives, and answers once it is started. A Run ID
        names the Run for good, and a start under one acts on what the Run index
        holds (ADR 0102): no such Run begins it, `new`; an interrupted Run, stopped,
        failed, or running with no live writer, resumes from its committed state at
        the next run epoch, `resumed`, as `run.resume` does; a finished Run is
        answered as it ended with no handler called, `ended`. A Run live with a
        writer, in this engine or another, fails with `run_in_use`; a Run started
        with other inputs or by another caller with `run_conflict` listing each
        difference; a Run the Run index lists whose state is missing or unreadable
        with `run_state_unavailable`, never starting afresh.

        Experimental: outside semver until it is promoted (ADR 0030).

        The Run is the Collie client package's own `Run` (ADR 0095).
        """
        warn_entry("run.start", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        warn_deprecated_settings(settings, DEPRECATED_SETTINGS)
        starting = functools.partial(
            self._connection.start,
            run_id=run_id,
            tags=None if tags is None else list(tags),
            limits=limits,
            settings=settings,
        )
        try:
            run = self._loop.call(starting)
        except LoopStopped:
            raise closed_error("run.start") from None
        return Run(self._loop, run)

    def start_recipe(
        self,
        recipe: str,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
        settings: RunSettings | None = None,
        recording: Recording | None = None,
        timeout: timedelta | NotGiven | None = not_given,
        detached: bool = False,
    ) -> Run:
        """Starts a Recipe's run, as `Collie.start_recipe`.

        Starts a Recipe's run, a Run with no handler in the program, and answers once
        it is started. Its Run ID works as `run.start`'s. With `detached`, the Run
        outlives the program that started it: closing its value frees only the
        value, `run.finish` waits for its end and `run.stop` stops it.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("recipe.start", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        warn_deprecated_settings(settings, DEPRECATED_SETTINGS)
        starting = functools.partial(
            self._connection.start_recipe,
            recipe,
            run_id=run_id,
            tags=None if tags is None else list(tags),
            limits=limits,
            settings=settings,
            recording=recording,
            timeout=timeout,
            detached=detached,
        )
        try:
            run = self._loop.call(starting)
        except LoopStopped:
            raise closed_error("recipe.start") from None
        return Run(self._loop, run)

    def start_catalog_run(
        self,
        target: CatalogTarget,
        *,
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        limits: Limits | None = None,
        settings: RunSettings | None = None,
        recording: Recording | None = None,
        timeout: timedelta | NotGiven | None = not_given,
        detached: bool = False,
    ) -> Run:
        """Starts a code-free catalog run, as `Collie.start_catalog_run`.

        Starts a code-free catalog run, a Run with no handler in the program whose
        verbs come from the catalog (ADR 0036): an Extraction of a page by a schema,
        `extract`, an Agent task, `agent`, or a Plan replayed with no model call,
        `plan_replay`; and answers once it is started. Its Run ID works as
        `run.start`'s. With `detached`, the Run outlives the program that started
        it: closing its value frees only the value, `run.finish` waits for its end
        and `run.stop` stops it.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("catalog_run.start", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        warn_deprecated_settings(settings, DEPRECATED_SETTINGS)
        starting = functools.partial(
            self._connection.start_catalog_run,
            target,
            run_id=run_id,
            tags=None if tags is None else list(tags),
            limits=limits,
            settings=settings,
            recording=recording,
            timeout=timeout,
            detached=detached,
        )
        try:
            run = self._loop.call(starting)
        except LoopStopped:
            raise closed_error("catalog_run.start") from None
        return Run(self._loop, run)

    def start_crawl(
        self,
        crawl: Crawl,
        *,
        seeds: Iterable[RequestSeed | SitemapSeed],
        run_id: str | None = None,
        tags: Iterable[str] | None = None,
        settings: CrawlSettings | None = None,
    ) -> CrawlRun:
        """Starts `crawl` from `seeds`, at least one, and serves its handlers.

        Starts a crawl from its seeds, and answers once its Run is started.
        The engine then leases each work item to the program's handler as a
        `crawl_request.handle` request, and the crawl's Run finishes once
        every work item is done, or, with `crawl.keep_alive`, once
        `run.finish` lets it and every work item is done. A Run ID works as
        `run.start`'s.

        As `AsyncFoxhound.start_crawl`, except that each work item's handler,
        a plain function, runs on the Connection's thread pool, so a handler
        that blocks holds no other handler back. Closing the Run cancels the
        call each handler has in flight, which raises `closed`, and waits
        for every handler to return (ADR 0093).

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry(
            "crawl.start",
            FOXHOUND_EXPERIMENTAL_OPERATIONS,
            FOXHOUND_DEPRECATED_OPERATIONS,
        )
        warn_deprecated_settings(settings, FOXHOUND_DEPRECATED_SETTINGS)
        starting = functools.partial(
            self._crawls.start_threaded,
            self._loop,
            crawl,
            seeds=list(seeds),
            run_id=run_id,
            tags=None if tags is None else list(tags),
            settings=settings,
        )
        try:
            return self._loop.call(starting)
        except LoopStopped:
            raise closed_error("crawl.start") from None

    def open_run(self, run_id: str) -> RunView:
        """Opens a view of a Run by its ID, as `Collie.open_run`."""
        opening = functools.partial(self._connection.open_run, run_id)
        try:
            view = self._loop.call(opening)
        except LoopStopped:
            raise closed_error("open_run") from None
        return RunView(view)

    def call(
        self,
        operation: str,
        params: Mapping[str, JsonValue] | None = None,
        *,
        timeout: timedelta | NotGiven | None = not_given,
    ) -> JsonValue:
        """Sends `operation`, as `AsyncFoxhound.call` does, and waits for its result."""
        sent = raw_params(operation, params, timeout)
        try:
            return self._loop.call(
                functools.partial(self._connection.raw, operation, sent)
            )
        except LoopStopped:
            raise closed_error(operation) from None

    def explain_settings(
        self, keys: Iterable[str] | None = None
    ) -> SettingsExplanation:
        """What the Connection runs with, as `Collie.explain_settings` says.

        Shows each setting's effective value, the layer that set it and the layer
        that locked it, from the built-in defaults, the Operator policy, user
        configuration, the project configuration given and the run layer: the
        Connection's own settings with the run values given over them, or a live
        Run's as it opened when `run_id` names one; and lists the keys only the
        other Product honours as that Product's.
        """
        explaining = functools.partial(
            self._connection.explain_settings, None if keys is None else list(keys)
        )
        try:
            return self._loop.call(explaining)
        except LoopStopped:
            raise closed_error("config.explain") from None

    def close(self) -> None:
        """Closes the Connection and joins its thread, as `Collie.close`; idempotent.

        Any later call fails with `closed`; a later close raises the first
        one's failure.
        """
        with self._close_lock:
            if not self._closed:
                try:
                    self._loop.call(self._connection.close)
                except LoopStopped:
                    # An interrupted close stopped the loop after the close ended.
                    pass
                except Exception as failure:
                    self._close_failure = failure
                finally:
                    self._closed = True
                    self._loop.stop()
        if self._close_failure is not None:
            raise self._close_failure


def async_connect_over(
    transport: ByteStream,
    *,
    max_listener_queue: int = MAX_LISTENER_QUEUE,
    settings: FoxhoundSettings | None = None,
) -> Connecting:
    """Opens an `AsyncFoxhound` over `transport`, with no process behind it.

    `max_listener_queue` and `settings` are `connect()`'s.
    """
    return Connecting(
        functools.partial(
            open_over,
            _FOXHOUND,
            transport,
            max_listener_queue=max_listener_queue,
            settings=settings,
        )
    )


def connect_over(
    transport: ByteStream,
    *,
    max_listener_queue: int = MAX_LISTENER_QUEUE,
    settings: FoxhoundSettings | None = None,
) -> Foxhound:
    """Opens a sync `Foxhound` over `transport`, with no process behind it.

    Its private loop drives `transport`, so a transport bound to another
    event loop does not fit. `max_listener_queue` and `settings` are
    `connect()`'s.
    """
    return _open_sync(
        functools.partial(
            open_over,
            _FOXHOUND,
            transport,
            max_listener_queue=max_listener_queue,
            settings=settings,
        )
    )


def _open_sync(opening: Callable[[], Awaitable[ClientConnection]]) -> Foxhound:
    """Opens a Connection on a private loop, stopped again when the opening fails."""
    loop = PrivateLoop()
    try:
        connection = loop.call(opening)
    except BaseException:
        loop.stop()
        raise
    return Foxhound(loop, connection)
