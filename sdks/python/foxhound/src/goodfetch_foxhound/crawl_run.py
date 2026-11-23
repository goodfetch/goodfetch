"""A crawl's Run (ADRs 0093, 0102 and 0103), and what serves its handlers.

`AsyncFoxhound.start_crawl` opens an `AsyncCrawlRun`, whose handlers the
engine reaches with `crawl_request.handle` requests, one per attempt at a
work item. Each attempt runs its Label's handler as a task beside the
Connection's, with an `AsyncCrawlContext` bound to its lease, and is
answered once the handler returns or raises. Closing the Run cancels every
attempt still running and waits until each is answered, so none is left
behind. `records(dataset)` reads a Dataset's committed Records as the engine
streams them, one per credit granted. TypeScript's `crawl-run.ts` is its
twin.
"""

from __future__ import annotations

import functools
import traceback
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any, Protocol, TypeAlias, cast

import anyio
from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.generated.errors import ClosedError, RunNotFoundError
from goodfetch_collie_client.generated.handles import AsyncPage
from goodfetch_collie_client.goodfetch_error import GoodfetchError
from goodfetch_collie_client.listeners import spawner
from goodfetch_collie_client.markers import warn_entry
from goodfetch_collie_client.run import AsyncRun
from goodfetch_collie_client.settings import settings_on_wire

from goodfetch_foxhound.crawl import (
    AsyncCrawlContext,
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
)

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Iterable

    from goodfetch_collie_client.connection import ClientConnection
    from goodfetch_collie_client.generated.models import RunStarted
    from goodfetch_collie_client.sequence import AsyncSequence
    from pydantic import BaseModel

    from goodfetch_foxhound.crawl import AsyncCrawl, Dataset
    from goodfetch_foxhound.generated.models import RequestSeed
    from goodfetch_foxhound.generated.settings import CrawlSettings

    _Handle: TypeAlias = Callable[
        [CrawlRequestHandleParams, "_Attempt"], Awaitable[dict[str, object]]
    ]
    """What runs one attempt on the crawl's Run its request names."""

__all__ = ["AsyncCrawlRun"]

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
        crawl: AsyncCrawl,
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
            attempt.scope.cancel()
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
        context = AsyncCrawlContext(
            self._request_of(request), self._attempt_of(request)
        )
        handles = cast("Callable[[AsyncCrawlContext[Any]], Awaitable[None]]", handler)
        with attempt.scope:
            try:
                await handles(context)
            except GoodfetchError:
                raise
            except Exception as error:
                raise handler_failed(label, error) from error
            return {}
        # Only closing the Run cancels the attempt's scope.
        raise self._stopped()

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

    def _attempt_of(self, request: CrawlRequestHandleParams) -> HandlerAttempt:
        """What the attempt's context calls the engine with, on this Run."""
        page_id = request.page_id
        return HandlerAttempt(
            run_id=self.id,
            crawl_request_id=request.crawl_request.id,
            lease_epoch=request.lease_epoch,
            crawl=self._crawl,
            operations=AsyncFoxhoundOperations(self._host),
            page=None if page_id is None else AsyncPage(self._host.child(page_id)),
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
        seeds: Iterable[RequestSeed],
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
        warn_entry("crawl.start", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
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
                "crawl.start",
                functools.partial(operations.crawl_start, params),
                functools.partial(self._opened, crawl),
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

    def _opened(self, crawl: AsyncCrawl, started: RunStarted) -> AsyncCrawlRun:
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
