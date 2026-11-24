"""A crawl's Run (ADRs 0093, 0102 and 0103), and what serves its handlers.

`AsyncFoxhound.start_crawl` opens an `AsyncCrawlRun`, whose handlers the
engine reaches with `crawl_request.handle` requests, one per attempt at a
work item. Each attempt runs its Label's handler as a task beside the
Connection's, with an `AsyncCrawlContext` bound to its lease, and is
answered once the handler returns or raises. `Foxhound.start_crawl` opens a
`CrawlRun`, the sync twin over an `AsyncCrawlRun` on the Connection's
private loop, whose plain-function handlers run on that loop's thread pool
with a `CrawlContext`. Closing the Run cancels every attempt still running,
and each call a sync handler has in flight, and waits until each attempt is
answered, so none is left behind. `records(dataset)` reads a Dataset's
committed Records as the engine streams them, one per credit granted.
TypeScript's `crawl-run.ts` is its twin.
"""

from __future__ import annotations

import functools
import traceback
import weakref
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any, Protocol, TypeAlias, cast

import anyio
import anyio.to_thread
from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.connection import closed_error
from goodfetch_collie_client.generated.errors import ClosedError, RunNotFoundError
from goodfetch_collie_client.generated.handles import AsyncPage, Page, Session
from goodfetch_collie_client.goodfetch_error import GoodfetchError
from goodfetch_collie_client.listeners import spawner
from goodfetch_collie_client.markers import warn_entry
from goodfetch_collie_client.portal import CallGroup, LoopStopped
from goodfetch_collie_client.run import AsyncRun, Run
from goodfetch_collie_client.sequence import Sequence
from goodfetch_collie_client.settings import settings_on_wire

from goodfetch_foxhound.crawl import (
    AsyncCrawlContext,
    Crawl,
    CrawlContext,
    CrawlRequest,
    HandlerAttempt,
    check_startable,
    crawl_declaration,
    handler_of,
    payload_of,
)
from goodfetch_foxhound.generated.errors import HandlerFailedError
from goodfetch_foxhound.generated.models import (
    CrawlRequestHandleParams,
    CrawlStartParams,
    DatasetRecord,
)
from goodfetch_foxhound.generated.operations import (
    DEPRECATED_OPERATIONS,
    EXPERIMENTAL_OPERATIONS,
    AsyncFoxhoundOperations,
    FoxhoundOperations,
)

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Iterable

    from goodfetch_collie_client.connection import ClientConnection
    from goodfetch_collie_client.generated.models import (
        PageId,
        RunId,
        RunStarted,
        SessionId,
    )
    from goodfetch_collie_client.handle import HandleHost
    from goodfetch_collie_client.portal import PrivateLoop
    from goodfetch_collie_client.sequence import AsyncSequence
    from pydantic import BaseModel

    from goodfetch_foxhound.crawl import AsyncCrawl, Dataset
    from goodfetch_foxhound.generated.models import RequestSeed, SitemapSeed
    from goodfetch_foxhound.generated.settings import CrawlSettings

    _Handle: TypeAlias = Callable[
        [CrawlRequestHandleParams, "_Attempt"], Awaitable[dict[str, object]]
    ]
    """What runs one attempt on the crawl's Run its request names."""

__all__ = ["AsyncCrawlRun", "CrawlRun"]

RECORD = "$/record"
"""The notification each Record `run.records` reads comes in."""


class _Served(Protocol):
    """A request the engine sent, which the Client answers once."""

    @property
    def params(self) -> object:
        """Its params, as the engine sent them."""
        ...

    async def serve(self, work: Callable[[], Awaitable[object]]) -> None:
        """Runs `work` and answers the engine with what it returns or raises."""
        ...


@dataclass(eq=False)
class _Attempt:
    """One attempt at a work item: what cancels it, and when it is answered."""

    scope: anyio.CancelScope = field(default_factory=anyio.CancelScope)
    """What closing its Run cancels its handler through."""
    answered: anyio.Event = field(default_factory=anyio.Event)
    """Set once its answer is written, or once it ended unanswered."""
    held_by: set[_Attempt] | None = None
    """The attempts of its Run, which hold it until it is answered."""
    calls: CallGroup | None = None
    """Its sync handler's calls, which run outside `scope`; none for an async one."""

    def cancel(self) -> None:
        """Cancels its handler, and each call its sync handler has in flight."""
        self.scope.cancel()
        if self.calls is not None:
            self.calls.cancel()

    def end(self) -> None:
        """Marks it answered, and lets its Run forget it."""
        if self.held_by is not None:
            self.held_by.discard(self)
        self.answered.set()


class AsyncCrawlRun(AsyncRun):
    """A crawl's Run, opened by `AsyncFoxhound.start_crawl` (ADR 0102).

    A program never constructs one. Leaving an `async with` block or
    `close()` stops each handler's attempt still running, then the Run.
    """

    def __init__(
        self,
        connection: ClientConnection,
        started: RunStarted,
        crawl: AsyncCrawl | Crawl,
        crawls: CrawlRuns,
    ) -> None:
        """Takes its Connection, its start's answer, its crawl and the crawls' Runs."""
        super().__init__(connection, started)
        self._crawl = crawl
        self._crawls = crawls
        self._attempts: set[_Attempt] = set()
        self._stopping = False
        crawls.hold(self, self._handle)

    def records(self, dataset: Dataset[Any]) -> AsyncSequence[DatasetRecord]:
        """The committed Records of `dataset`, read with `async for` (ADR 0103).

        Reads the committed Records of one Dataset of a crawl's Run in number
        order, from the first, or after the number `after`, which resumes a
        reader from the last Record it handled. Each Record goes in a
        `$/record` notification with its number, its value, its request's ID
        and URL and when it was committed, one per credit granted with
        `$/credit`. While the Run is live it follows the Run, sending each
        Record once it commits; once the Run has ended, or has no live writer,
        and every Record is sent, it answers how many it sent and the last
        one's number. With `through`, such as the number `dataset.page`
        answers as the last committed, it reads no Record numbered past it
        and answers once it sent every Record through it, a live Run's
        included. Records are read from the Run's committed state on a
        connection of their own, so a reader that grants no credit never
        slows the crawl. With `wall` set to `true` it reads only the Records
        whose page was read past a Wall, which their Provenance marks, and
        with `false` only the others (ADR 0052). A Dataset the crawl does not
        declare fails with `dataset_unknown`.

        This member reads every Record from the first; `after`, `through`
        and `wall` are not taken yet.

        Experimental: outside semver until it is promoted (ADR 0030).

        Its stream is sent when it is called; leaving an `async with` block,
        `aclose()` or releasing it cancel it at once.

        Raises:
            ClosedError: The Run is closed; nothing is sent.
            RuntimeError: No asyncio or Trio loop runs on this thread.
        """
        warn_entry("run.records", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        params = {"run_id": self.id, "dataset": dataset.name}
        return self._read("run.records", params, _record_of)

    async def _close(self) -> None:
        """Cancels each attempt still running and waits for its answer, then closes.

        A request leased after the close began is answered `closed`.
        """
        self._stopping = True
        attempts = list(self._attempts)
        for attempt in attempts:
            attempt.cancel()
        for attempt in attempts:
            await attempt.answered.wait()
        try:
            await super()._close()
        finally:
            await self._crawls.forget(self)

    async def _handle(
        self, request: CrawlRequestHandleParams, attempt: _Attempt
    ) -> dict[str, object]:
        """Runs the handler of the request's Label on its attempt.

        Raises:
            HandlerFailedError: The Label has no handler, or the handler
                raised an exception of its own.
            ClosedError: The Run is closed, which stopped the attempt.
        """
        label = request.crawl_request.label
        handler = handler_of(self._crawl, label)
        if handler is None:
            message = f"the Crawl attaches no handler to the Label {label}"
            raise HandlerFailedError(
                message,
                fix="Attach one with `@crawl.handle(label)` before starting the crawl.",
                details={"label": label},
            )
        if self._stopping:
            raise self._stopped()
        self._attempts.add(attempt)
        attempt.held_by = self._attempts
        with attempt.scope:
            try:
                await self._run_handler(handler, request, attempt)
            except GoodfetchError:
                raise
            except Exception as error:
                raise handler_failed(label, error) from error
            return {}
        # Only closing the Run cancels the attempt's scope.
        raise self._stopped()

    async def _run_handler(
        self,
        handler: Callable[..., object],
        request: CrawlRequestHandleParams,
        attempt: _Attempt,
    ) -> None:
        """Runs an async handler here, or a sync one on a worker thread."""
        crawl_request = self._request_of(request)
        lease = self._lease_of(request)
        page_id = request.page_id
        if not isinstance(self._crawl, Crawl):
            page = (
                None if page_id is None else self._page_of(page_id, request.session.id)
            )
            operations = AsyncFoxhoundOperations(self._host)
            context = AsyncCrawlContext(crawl_request, lease, operations, page)
            await cast("Callable[[AsyncCrawlContext[Any]], Awaitable[None]]", handler)(
                context
            )
            return
        threaded = _threaded.get(self)
        if threaded is None:
            message = "invariant: a sync crawl's Run has its `CrawlRun`"
            raise RuntimeError(message)
        sync_page = (
            None if page_id is None else threaded.page_of(page_id, request.session.id)
        )
        sync_context = CrawlContext(
            crawl_request, lease, FoxhoundOperations(threaded.host), sync_page
        )
        # The thread's calls run on the loop outside the attempt's scope,
        # which waits for the thread, so closing the Run cancels them apart.
        attempt.calls = CallGroup(self._stopped)
        with attempt.calls.bound():
            await anyio.to_thread.run_sync(
                cast("Callable[[CrawlContext[Any]], None]", handler), sync_context
            )

    def _page_of(self, page_id: PageId, session_id: SessionId) -> AsyncPage:
        """The page of ID `page_id` the engine lends a handler, in its serving Session.

        That Session, of ID `session_id`, is the one the Run keeps of that ID,
        which its other pages there share (ADR 0092).
        """
        host, session = self._kept_session(session_id)
        return AsyncPage(host.child(page_id), session)

    def _request_of(self, request: CrawlRequestHandleParams) -> CrawlRequest[Any]:
        """The crawl request the handler reads, its Payload typed by its Label."""
        crawl_request = request.crawl_request
        return CrawlRequest(
            id=crawl_request.id,
            url=crawl_request.url,
            label=crawl_request.label,
            method=crawl_request.method.root,
            payload=payload_of(self._crawl, crawl_request.label, crawl_request.payload),
        )

    def _lease_of(self, request: CrawlRequestHandleParams) -> HandlerAttempt:
        """The lease the attempt's context calls the engine with, on this Run."""
        return HandlerAttempt(
            run_id=self.id,
            crawl_request_id=request.crawl_request.id,
            lease_epoch=request.lease_epoch,
            crawl=self._crawl,
            page_id=request.page_id,
        )

    def _stopped(self) -> ClosedError:
        """The `closed` an attempt the Run's close stopped is answered with."""
        fix = (
            "Keep the Run open until its crawl is done, or start it again by its"
            " Run ID."
        )
        return ClosedError(
            f"the Run {self.id} is closed, which stopped the handler's attempt",
            fix=fix,
        )


class CrawlRun(Run):
    """A crawl's Run of the sync API, opened by `Foxhound.start_crawl` (ADR 0102).

    Any number of threads may share it. A program never constructs one.
    Leaving a `with` block or `close()` stops each handler's attempt still
    running, its call in flight included, then the Run.
    """

    def __init__(self, loop: PrivateLoop, run: AsyncCrawlRun) -> None:
        """Takes its Connection's private loop and the crawl's Run it drives there.

        Built on the loop as the Run opens, before the engine leases any
        work item to its handlers, which call through it.
        """
        super().__init__(loop, run)
        self._crawl_run = run
        _threaded[run] = _Threaded(self._host, self._sessions)

    def records(self, dataset: Dataset[Any]) -> Sequence[DatasetRecord]:
        """The committed Records of `dataset`, read with `for`, as the async Run's.

        Reads the committed Records of one Dataset of a crawl's Run in number
        order, from the first, or after the number `after`, which resumes a
        reader from the last Record it handled. Each Record goes in a
        `$/record` notification with its number, its value, its request's ID
        and URL and when it was committed, one per credit granted with
        `$/credit`. While the Run is live it follows the Run, sending each
        Record once it commits; once the Run has ended, or has no live writer,
        and every Record is sent, it answers how many it sent and the last
        one's number. With `through`, such as the number `dataset.page`
        answers as the last committed, it reads no Record numbered past it
        and answers once it sent every Record through it, a live Run's
        included. Records are read from the Run's committed state on a
        connection of their own, so a reader that grants no credit never
        slows the crawl. With `wall` set to `true` it reads only the Records
        whose page was read past a Wall, which their Provenance marks, and
        with `false` only the others (ADR 0052). A Dataset the crawl does not
        declare fails with `dataset_unknown`.

        This member reads every Record from the first; `after`, `through`
        and `wall` are not taken yet.

        Experimental: outside semver until it is promoted (ADR 0030).

        Any thread may read it. Leaving a `with` block, `close()` or
        releasing it cancel its read at once.

        Raises:
            ClosedError: The Run or its Connection is closed; nothing is sent.
        """
        warn_entry("run.records", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        run = self._crawl_run

        async def reading() -> AsyncSequence[DatasetRecord]:
            return run.records(dataset)

        try:
            sequence = self._loop.call(reading)
        except LoopStopped:
            raise closed_error("run.records") from None
        return Sequence(self._loop, sequence, lambda: closed_error("run.records"))


@dataclass(frozen=True)
class _Threaded:
    """What a sync crawl's handlers reach its Run through, from the loop.

    It holds the `CrawlRun`'s host and the Sessions it keeps by ID, not the
    `CrawlRun`, which holds the async Run that keys it in `_threaded`.
    """

    host: HandleHost[RunId]
    """The sync host its handlers call through."""
    sessions: dict[str, tuple[HandleHost[SessionId], Session]]
    """The Sessions the `CrawlRun` keeps by ID, which its pages share."""

    def page_of(self, page_id: PageId, session_id: SessionId) -> Page:
        """The page of ID `page_id` the engine lends a handler, in its serving Session.

        That Session, of ID `session_id`, is the one the `CrawlRun` keeps of
        that ID, which its other pages there share (ADR 0092).
        """
        kept = self.sessions.get(session_id.root)
        if kept is None:
            host = self.host.child(session_id)
            kept = (host, Session(host))
            self.sessions[session_id.root] = kept
        host, session = kept
        return Page(host.child(page_id), session)


_threaded: weakref.WeakKeyDictionary[AsyncCrawlRun, _Threaded] = (
    weakref.WeakKeyDictionary()
)
"""What the handlers of each Run a `CrawlRun` drives reach it through."""


class CrawlRuns:
    """The crawls' Runs started on one Connection, which its handlers serve.

    It serves the engine's `crawl_request.handle` requests: each goes to the
    Run its `run_id` names, once that Run's start is answered.
    """

    def __init__(self, connection: ClientConnection) -> None:
        """Serves `crawl_request.handle` on `connection`."""
        self._connection = connection
        self._runs: dict[str, tuple[AsyncCrawlRun, _Handle]] = {}
        self._starting: set[anyio.Event] = set()
        self._reading: tuple[anyio.CancelScope, anyio.Event] | None = None
        """What stops the read its crawls' Runs hold, and when it stopped."""
        connection.serve("crawl_request.handle", self._served)

    async def start(
        self,
        crawl: AsyncCrawl,
        *,
        seeds: Iterable[RequestSeed | SitemapSeed],
        run_id: str | None,
        tags: Iterable[str] | None,
        settings: CrawlSettings | None,
    ) -> AsyncCrawlRun:
        """Starts `crawl` from `seeds`: `AsyncFoxhound.start_crawl`.

        Raises:
            LabelUnhandledError: A Label has no handler; nothing is sent.
            LabelUnknownError: A seed names a Label `crawl` does not declare.
            ClosedError: The Connection is closed; nothing is sent.
        """
        return await self._start(
            crawl,
            functools.partial(self._opened, crawl),
            seeds=seeds,
            run_id=run_id,
            tags=tags,
            settings=settings,
        )

    async def start_threaded(
        self,
        loop: PrivateLoop,
        crawl: Crawl,
        *,
        seeds: Iterable[RequestSeed | SitemapSeed],
        run_id: str | None,
        tags: Iterable[str] | None,
        settings: CrawlSettings | None,
    ) -> CrawlRun:
        """Starts the sync `crawl` from `seeds` on `loop`: `Foxhound.start_crawl`.

        Raises:
            LabelUnhandledError: A Label has no handler; nothing is sent.
            LabelUnknownError: A seed names a Label `crawl` does not declare.
            ClosedError: The Connection is closed; nothing is sent.
        """
        threaded: list[CrawlRun] = []

        def opened(started: RunStarted) -> AsyncCrawlRun:
            run = self._opened(crawl, started)
            threaded.append(CrawlRun(loop, run))
            return run

        await self._start(
            crawl, opened, seeds=seeds, run_id=run_id, tags=tags, settings=settings
        )
        return threaded[0]

    async def _start(
        self,
        crawl: AsyncCrawl | Crawl,
        opened: Callable[[RunStarted], AsyncCrawlRun],
        *,
        seeds: Iterable[RequestSeed | SitemapSeed],
        run_id: str | None,
        tags: Iterable[str] | None,
        settings: CrawlSettings | None,
    ) -> AsyncCrawlRun:
        """Sends `crawl.start`, and `opened` builds the Run it answers."""
        sent_seeds = list(seeds)
        check_startable(crawl, sent_seeds)
        labels, datasets = crawl_declaration(crawl)
        fields: dict[str, object] = {
            "labels": {name: _wire(label) for name, label in labels.items()}
        }
        if datasets:
            fields["datasets"] = [_wire(dataset) for dataset in datasets]
        fields["seeds"] = [_wire(sent) for sent in sent_seeds]
        if run_id is not None:
            fields["run_id"] = run_id
        if tags is not None:
            fields["tags"] = list(tags)
        if settings is not None:
            fields["settings"] = settings_on_wire(settings)
        # The start carries the project file `connect()` read (ADR 0083).
        project = self._connection.project
        if project is not None:
            fields["project"] = project
        params = CrawlStartParams.model_construct(None, **fields)
        operations = AsyncFoxhoundOperations(self._connection)
        starting = anyio.Event()
        self._starting.add(starting)
        try:
            return await self._connection.start_run(
                "crawl.start", functools.partial(operations.crawl_start, params), opened
            )
        finally:
            self._starting.discard(starting)
            starting.set()

    async def forget(self, run: AsyncCrawlRun) -> None:
        """Forgets a closed crawl's Run; a later start of its Run ID holds its own.

        Once no crawl's Run is left, it stops reading between calls.
        """
        held = self._runs.get(run.id)
        if held is not None and held[0] is run:
            del self._runs[run.id]
        if self._runs or self._reading is None:
            return
        scope, ended = self._reading
        self._reading = None
        scope.cancel()
        await ended.wait()

    def hold(self, run: AsyncCrawlRun, handle: _Handle) -> None:
        """Holds a crawl's Run just opened, whose attempts `handle` runs.

        While it holds one, it reads the engine's messages between calls.
        """
        self._runs[run.id] = (run, handle)
        if self._reading is None:
            self._reading = (anyio.CancelScope(), anyio.Event())
            spawner()(functools.partial(self._read, *self._reading))

    def _opened(self, crawl: AsyncCrawl | Crawl, started: RunStarted) -> AsyncCrawlRun:
        return AsyncCrawlRun(self._connection, started, crawl, self)

    async def _read(self, scope: anyio.CancelScope, ended: anyio.Event) -> None:
        """Reads the engine's messages while no call does, until `scope` is cancelled.

        A crawl's handlers are leased whether or not the program waits on a
        call, so a request the engine sends between two calls is served
        then, not at the program's next call.
        """
        try:
            with scope:
                await self._connection.keep_reading()
        finally:
            ended.set()

    def _served(self, request: _Served) -> None:
        """Starts serving `request` beside the Connection's tasks."""
        spawner()(functools.partial(self._serve, request))

    async def _serve(self, request: _Served) -> None:
        attempt = _Attempt()
        try:
            await request.serve(
                functools.partial(self._handle, request.params, attempt)
            )
        finally:
            attempt.end()

    async def _handle(self, params: object, attempt: _Attempt) -> dict[str, object]:
        """Hands the request to the Run it names, once its start is answered.

        Raises:
            RunNotFoundError: No crawl started on this Connection runs it.
        """
        request = result_of(CrawlRequestHandleParams, params)
        run_id = request.run_id.root
        if run_id not in self._runs:
            # Its start's answer may not be read yet.
            for starting in list(self._starting):
                await starting.wait()
        held = self._runs.get(run_id)
        if held is None:
            message = (
                f"the crawl request {request.crawl_request.id} names the Run "
                f"{run_id}, which no crawl started on this Connection runs"
            )
            raise RunNotFoundError(message, details={"run_id": run_id})
        _, handle = held
        return await handle(request, attempt)


def handler_failed(label: str, error: Exception) -> HandlerFailedError:
    """The `handler_failed` of a handler that raised `error`, its own (ADR 0109).

    The message names the exception's type and message, and the details
    carry them with the frames of its traceback.
    """
    kind = type(error).__name__
    message = str(error)
    return HandlerFailedError(
        f"the handler of the Label {label} failed with {kind}: {message}",
        details={
            "exception_type": kind,
            "exception_message": message,
            "exception_stack": "".join(traceback.format_tb(error.__traceback__)),
        },
    )


def _record_of(method: str, params: object) -> DatasetRecord | None:
    """The Record a `$/record` notification carries; none for another."""
    if method != RECORD or not isinstance(params, dict):
        return None
    record = cast("dict[str, object]", params).get("record")
    return None if record is None else result_of(DatasetRecord, record)


def _wire(model: BaseModel) -> dict[str, object]:
    """`model` as the wire spells it, a field left unset left out."""
    return model.model_dump(
        mode="json", by_alias=True, exclude_unset=True, warnings=False
    )
