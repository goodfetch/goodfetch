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
from typing import (
    TYPE_CHECKING,
    Any,
    Generic,
    Literal,
    Protocol,
    TypeAlias,
    TypeVar,
    cast,
    final,
    overload,
)

import anyio
import anyio.to_thread
from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.connection import closed_error
from goodfetch_collie_client.generated.errors import ClosedError, RunNotFoundError
from goodfetch_collie_client.generated.handles import AsyncPage, Page, Session
from goodfetch_collie_client.generated.models import (
    RunFinishParams,
    RunParams,
    RunStarted,
    RunStopParams,
)
from goodfetch_collie_client.goodfetch_error import GoodfetchError
from goodfetch_collie_client.handle import NotGiven, not_given, timeout_of
from goodfetch_collie_client.limits import limit_list
from goodfetch_collie_client.listeners import CREDITS, REGRANT, spawner
from goodfetch_collie_client.markers import warn_entry, warn_experimental
from goodfetch_collie_client.portal import CallGroup, LoopStopped
from goodfetch_collie_client.run import AsyncRun, Run
from goodfetch_collie_client.sequence import Sequence
from goodfetch_collie_client.settings import settings_on_wire
from pydantic import BaseModel, ConfigDict

from goodfetch_foxhound.crawl import (
    AsyncAttemptRun,
    AsyncCrawlContext,
    AttemptRun,
    Crawl,
    CrawlContext,
    CrawlRequest,
    DatasetRecord,
    HandlerAttempt,
    Label,
    build_seed_for_crawl,
    check_startable,
    crawl_declaration,
    handler_of,
    payload_of,
    read_record,
)
from goodfetch_foxhound.export import AsyncExport, Export
from goodfetch_foxhound.generated.errors import HandlerFailedError, RecordInvalidError
from goodfetch_foxhound.generated.models import (
    CrawlDeclaration,
    CrawlExtraction,
    CrawlRequestHandleParams,
    CrawlRequestOutcome,
    CrawlRequestOutcomes,
    CrawlRunProgress,
    CrawlRunReport,
    CrawlStartParams,
    DatasetList,
    DatasetListParams,
    DatasetPageParams,
    Enqueued,
    ExportCompression,
    ExportFormat,
    FailedRequests,
    Now,
    OutcomesAfter,
    OutcomesAfter1,
    RejectedRecord,
    RejectedRecords,
    RetriedFailed,
    RunAddSeedParams,
    RunAddSeedsParams,
    RunExportParams,
    RunFailedRequestsParams,
    RunOutcomesParams,
    RunRejectedRecordsParams,
    RunRetryFailedParams,
    SeedAttempt,
)
from goodfetch_foxhound.generated.models import (
    Export as WireExport,
)
from goodfetch_foxhound.generated.operations import (
    DEPRECATED_OPERATIONS,
    EXPERIMENTAL_OPERATIONS,
    AsyncFoxhoundOperations,
    FoxhoundOperations,
)
from goodfetch_foxhound.seed_stream import SeedStreamReading

if TYPE_CHECKING:
    from collections.abc import AsyncIterable, Awaitable, Callable, Iterable, Mapping
    from datetime import timedelta

    from goodfetch_collie_client.connection import ClientConnection
    from goodfetch_collie_client.generated.models import (
        KeyValueStoreAttempt,
        PageId,
        RunId,
        SessionId,
    )
    from goodfetch_collie_client.handle import HandleHost
    from goodfetch_collie_client.json_values import JSONObject
    from goodfetch_collie_client.key_value_store import KeyValueStore
    from goodfetch_collie_client.limits import Limits
    from goodfetch_collie_client.portal import PrivateLoop
    from goodfetch_collie_client.sequence import AsyncSequence

    from goodfetch_foxhound.crawl import AsyncCrawl, Dataset
    from goodfetch_foxhound.generated.models import RequestSeed, SitemapSeed
    from goodfetch_foxhound.generated.open_values import OutcomeKind
    from goodfetch_foxhound.generated.settings import CrawlSettings

    _Handle: TypeAlias = Callable[
        [CrawlRequestHandleParams, "_Attempt"], Awaitable[dict[str, object]]
    ]
    """What runs one attempt on the crawl's Run its request names."""

__all__ = ["AsyncCrawlRun", "CodeFreeCrawl", "CrawlRun"]

CodeFreeCrawl: TypeAlias = CrawlDeclaration | CrawlExtraction
"""A crawl with no handler in the program (ADR 0093).

A reviewed `CrawlDeclaration`, whose Labels follow links and extract
Records into its Datasets, or a code-free `CrawlExtraction`, its one-Label
shorthand (the Label `page` and the Dataset `pages`).
"""

RECORD = "$/record"
"""The notification each Record `run.records` reads comes in."""

R = TypeVar("R")
"""What a Dataset's Records read as."""

M = TypeVar("M", bound=BaseModel)
"""What a Pydantic Payload model types as."""

_Item = TypeVar("_Item")
"""What a read hands the program, one at a time."""


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
        crawl: AsyncCrawl | Crawl | None,
        crawls: CrawlRuns,
        *,
        seeds: list[Enqueued] | None = None,
        detached: bool = False,
    ) -> None:
        """Takes its Connection, its start's answer, its crawl and the crawls' Runs.

        A code-free crawl has no crawl of the program's, and a `detached`
        start's Run outlives its program (ADR 0102).
        """
        super().__init__(connection, started, detached=detached)
        self._client = connection
        self._crawl = crawl
        self._crawls = crawls
        self._seeds: list[Enqueued] = list(seeds or [])
        self._attempts: set[_Attempt] = set()
        self._stopping = False
        crawls.hold(self, self._handle)

    def seeds(self) -> list[Enqueued]:
        """Each seed's original answer in order, with no round trip (ADR 0102).

        Lists each seed's answer, in the order the crawl's start gave them, as
        the Frontier answered it when the Run began: added, with its crawl
        request's ID and its URL as parsed, or not added, with its reason
        (`duplicate`, `domain_policy`, `unsupported_scheme`, `invalid_url` or a
        cap's name). A relative seed is `invalid_url`, since nothing gives it a
        base. So a crawl that does nothing says why. Read from the Run's
        committed state, so it works on a Run no program holds.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        return list(self._seeds)

    @overload
    async def add_seed(self, seed_or_url: RequestSeed | SitemapSeed) -> Enqueued: ...
    @overload
    async def add_seed(
        self,
        seed_or_url: str,
        label: Label[None] | str,
        *,
        method: str | None = None,
        body: bytes | None = None,
        headers: Mapping[str, str] | None = None,
        priority: int | None = None,
        forefront: bool | None = None,
        key: str | None = None,
    ) -> Enqueued: ...
    @overload
    async def add_seed(
        self,
        seed_or_url: str,
        label: Label[M],
        payload: M,
        *,
        method: str | None = None,
        body: bytes | None = None,
        headers: Mapping[str, str] | None = None,
        priority: int | None = None,
        forefront: bool | None = None,
        key: str | None = None,
    ) -> Enqueued: ...
    @overload
    async def add_seed(
        self,
        seed_or_url: str,
        label: Label[JSONObject],
        payload: JSONObject,
        *,
        method: str | None = None,
        body: bytes | None = None,
        headers: Mapping[str, str] | None = None,
        priority: int | None = None,
        forefront: bool | None = None,
        key: str | None = None,
    ) -> Enqueued: ...
    async def add_seed(
        self,
        seed_or_url: RequestSeed | SitemapSeed | str,
        label: Label[Any] | str | None = None,
        payload: object = None,
        *,
        method: str | None = None,
        body: bytes | None = None,
        headers: Mapping[str, str] | None = None,
        priority: int | None = None,
        forefront: bool | None = None,
        key: str | None = None,
    ) -> Enqueued:
        """Adds one seed to the running crawl (`run.add_seed`, ADR 0102).

        Adds one seed to a running crawl, `run.addSeed`, admitted and answered
        as soon as the Run state accepts it (written with the next Run-state
        batch at the default durability, or committed before answering at
        `full`), and answers as an enqueue does: added, with its crawl request's
        ID and its URL as parsed, or not added, with its reason (`duplicate`,
        `unsupported_scheme`, `invalid_url` or a cap's name). A relative seed is
        `invalid_url`. An idle `crawl.keep_alive` crawl takes it and goes on.
        Through a handler's context, naming its `attempt`, it is staged with
        that attempt: once the attempt ended it fails with `attempt_ended`. A
        Run this engine does not run as a live crawl, or a crawl already ending
        its Run, fails with `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        candidate = build_seed_for_crawl(
            self._crawl,
            seed_or_url,
            label,
            payload,
            method,
            body,
            headers,
            priority,
            forefront,
            key,
        )
        return await self._add_seed(candidate, attempt=None)

    def add_seeds(
        self,
        seeds: Iterable[RequestSeed | SitemapSeed]
        | AsyncIterable[RequestSeed | SitemapSeed],
    ) -> AsyncSequence[Enqueued]:
        """Streams seeds to the running crawl (`run.add_seeds`, ADRs 0024 and 0102).

        Takes a stream of seeds for a running crawl, `run.addSeeds`, and answers
        each as `run.add_seed` does, in order, so a million seeds never sit in
        memory. The engine grants the call 256 credits with `$/credit`, one seed
        each; the Client sends seeds in `$/seeds {id, seeds, end?}` batches,
        never more than its credits, and the engine admits each batch (staged
        with `attempt` when named, or written with the next Run-state batch at
        default durability), sends its answers in one `$/enqueued {id, answers}`
        and grants as many credits again. Once it answered the batch marked
        `end`, the call answers how many seeds were added and how many were not.
        Cancelling it keeps the seeds already answered. A batch beyond the
        credits granted, or a seed that is no seed, fails the call with
        `invalid_params`; a Run this engine does not run as a live crawl, or a
        crawl already ending its Run, fails with `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        return self._add_seeds(seeds, attempt=None)

    async def _add_seed(
        self, candidate: RequestSeed | SitemapSeed, *, attempt: SeedAttempt | None
    ) -> Enqueued:
        warn_entry("run.add_seed", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        fields: dict[str, object] = {"run_id": self._host.id, "seed": _wire(candidate)}
        if attempt is not None:
            fields["attempt"] = _wire(attempt)
        params = RunAddSeedParams.model_validate(fields)
        answer = await self._host.call("run.add_seed", params)
        return result_of(Enqueued, answer)

    def _add_seeds(
        self,
        seeds: Iterable[RequestSeed | SitemapSeed]
        | AsyncIterable[RequestSeed | SitemapSeed],
        *,
        attempt: SeedAttempt | None,
    ) -> AsyncSequence[Enqueued]:
        warn_entry("run.add_seeds", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        fields: dict[str, object] = {"run_id": self._host.id}
        if attempt is not None:
            fields["attempt"] = _wire(attempt)
        params = RunAddSeedsParams.model_validate(fields)
        crawl = self._crawl
        return self._open_sequence(
            "run.add_seeds",
            lambda connection, order: SeedStreamReading(
                connection, params, crawl, seeds, order
            ),
        )

    def records(
        self, dataset: Dataset[R], *, after: int | None = None
    ) -> AsyncSequence[DatasetRecord[R]]:
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

        Each Record's `value` is the Dataset's own type, parsed by its model
        with each field the model does not declare kept in `model_extra`; a
        Record the model refuses ends the read with `RecordInvalidError`
        naming its `number`, past which `after` resumes, once the Records
        before it are read. `through` and `wall` are not taken yet.

        Experimental: outside semver until it is promoted (ADR 0030).

        Its stream is sent when it is called; leaving an `async with` block,
        `aclose()` or releasing it cancel it at once.

        Raises:
            ClosedError: The Run is closed; nothing is sent.
            RuntimeError: No asyncio or Trio loop runs on this thread.
        """
        warn_entry("run.records", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        params: dict[str, object] = {"run_id": self.id, "dataset": dataset.name}
        if after is not None:
            params["after"] = after

        def item_of(method: str, notified: object) -> DatasetRecord[R] | None:
            record = _record_of(method, notified)
            return None if record is None else read_record(dataset, record)

        return self._read("run.records", params, item_of)

    def rejected_records(
        self, dataset: Dataset[Any], *, after: int | None = None
    ) -> AsyncSequence[RejectedRecord]:
        """The rejected Records of `dataset`, read with `async for` (ADRs 0103, 0104).

        Reads one Dataset's rejection table from a crawl's committed state:
        each Record rejected rather than committed, numbered from 1 in commit
        order, with the Record as JSON, the reason, each issue's path, the
        Hook when one rejected it, and the request's ID and URL. It starts at
        the first entry, or after `after`, the number a reader last handled.
        While the Run is live and nothing follows, it waits for the next
        commit; a page says `ended` once the Run has ended, or has no live
        writer, and no entry follows. With `through`, such as the `rejected`
        count `dataset.list` answers, it reads no entry numbered past it and
        never waits, a live Run's included: a page says `ended` once it
        reaches that number or has read every committed entry. A reader never
        slows the Run. A Dataset the crawl does not declare fails with
        `dataset_unknown`.

        Each entry's `data` is the Record's JSON as it was pushed, never
        parsed. The read goes page after page: its first page is asked when
        it is called, each next one once the program has read on, and it ends
        once a page says the table is read to its end. `through` is not taken
        here: a handler's `ctx.run` read bounds itself.

        Experimental: outside semver until it is promoted (ADR 0030).

        Leaving an `async with` block, `aclose()` or releasing it cancel the
        read at once.

        Raises:
            ClosedError: The Run is closed; nothing is sent.
            RuntimeError: No asyncio or Trio loop runs on this thread.
        """
        return self._rejected(dataset, after, committed=False)

    async def export(
        self,
        dataset: Dataset[Any],
        *,
        format: ExportFormat | str,
        compression: ExportCompression | str | None = None,
        wall: bool | None = None,
        settings: CrawlSettings | None = None,
    ) -> AsyncExport:
        """Exports `dataset`'s committed Records into an `AsyncExport` (ADR 0103).

        Writes an Export of one Dataset of a crawl's Run: the Records it
        committed when the call is made, in number order, in the format named,
        as its `format` value says: a table (`csv`, `xlsx`, `html_table`,
        `markdown`, and typed, `parquet`, `avro` and `arrow_ipc`) has one
        column per field seen in any of its Records, in first-seen order, and
        one row per Record. Every text Export is UTF-8 with no BOM unless
        `export.csv.bom` puts one on CSV, characters written as themselves (a
        character XML 1.0 cannot hold as U+FFFD in XML and feeds) and each
        value as it was committed: a table cell holds a string as its text, a
        number, `true` or `false` as JSON writes it, nothing for `null` or a
        field its Record lacks, and an object or array as its JSON text, or, in
        CSV under `export.csv.nested: flatten`, an object's fields in dotted
        columns. A Record whose page was read past a Wall carries its Wall
        marker, `{kind, vendor?}` as its Provenance holds it, as a `#wall`
        field in every format but the feeds, where it is a category of the
        scheme `urn:goodfetch:wall` naming its kind; `wall` set to `true`
        writes only those Records, `false` only the others. `compression`
        `gzip` or `zstd` compresses the whole file. Its `settings` and
        `project` are the run layer and the project configuration its
        `export.*` settings resolve under; a value the format cannot take fails
        with `setting_invalid`. Answers the Export, with its Record count, its
        last Record's number, its size, its SHA-256 and the run epoch of the
        Run's state it was written from, only once its file is whole in the
        Run's folder, where `export.save_as` and `export.stream` read it.
        Records committed after the call starts are not in it. A write the
        engine's storage refuses fails with `file_write_failed`, side `engine`,
        leaving no unfinished file, as does a call while that storage is at the
        `critical` level, before any byte is written. A Dataset the crawl does
        not declare fails with `dataset_unknown`; an `xlsx` sheet past Excel's
        limits, with `sheet_capacity_exceeded`.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            DatasetUnknownError: `dataset` is not declared by this crawl.
            SettingInvalidError: A setting is not valid for the chosen format.
            SheetCapacityExceededError: An `xlsx` sheet passes Excel's limits.
            FileWriteFailedError: The engine's storage refused the write.
            ClosedError: The Run is closed; nothing is sent.
        """
        warn_entry("run.export", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        fields: dict[str, object] = {
            "run_id": self._host.id,
            "dataset": dataset.name,
            "format": ExportFormat(format),
        }
        if compression is not None:
            fields["compression"] = ExportCompression(compression)
        if wall is not None:
            fields["wall"] = wall
        if settings is not None:
            fields["settings"] = settings_on_wire(settings)
        project = self._client.project
        if project is not None:
            fields["project"] = project
        params = RunExportParams.model_validate(fields)
        answer = await self._host.call("run.export", params)
        descriptor = result_of(WireExport, answer)
        return AsyncExport(self._client, descriptor, self._read)

    def outcomes(
        self,
        *,
        kinds: Iterable[OutcomeKind] | None = None,
        labels: Iterable[str | Label[Any]] | None = None,
        codes: Iterable[str] | None = None,
        after: int | Literal["now"] | None = None,
    ) -> AsyncSequence[CrawlRequestOutcome]:
        """A log of endings, not current state; retries supersede by `request.id`.

        Reads a crawl's outcomes from its committed state, in commit order: from the
        first, after a position, or with `after: "now"` from the next commit. Its
        filters, `kinds`, `labels` and `codes` (an Error code any of the request's
        failed attempts failed with), run in the engine. While the Run is live and
        nothing new matches, it waits for the next commit; a page says `ended` once
        the Run has ended, or has no live writer, and nothing more matches. A reader
        never slows the Run. A request `run.retry_failed` re-admits ends again, and
        its later outcome supersedes the earlier one by `request.id`.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            ClosedError: The Run is closed; nothing is sent.
            RuntimeError: No asyncio or Trio loop runs on this thread.
        """
        warn_entry("run.outcomes", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        host = self._host
        kind_list: list[OutcomeKind] | None = None if kinds is None else list(kinds)
        label_list = (
            None
            if labels is None
            else [lbl.name if isinstance(lbl, Label) else lbl for lbl in labels]
        )
        code_list = None if codes is None else list(codes)
        initial_after: OutcomesAfter | None = (
            None
            if after is None
            else (
                OutcomesAfter(Now.now)
                if after == "now"
                else OutcomesAfter(OutcomesAfter1(after))
            )
        )

        async def read(position: int | None) -> CrawlRequestOutcomes:
            cursor = (
                initial_after
                if position is None
                else OutcomesAfter(OutcomesAfter1(position))
            )
            params = RunOutcomesParams(
                run_id=host.id,
                kinds=kind_list,
                labels=label_list,
                codes=code_list,
                after=cursor,
            )
            answer = await host.call("run.outcomes", params)
            return result_of(CrawlRequestOutcomes, answer)

        return self._paged(
            "run.outcomes",
            read,
            items=lambda page: page.outcomes,
            position=lambda page: page.next,
            last=lambda page: page.ended,
            credits=CREDITS,
            regrant=REGRANT,
        )

    async def failed_requests(
        self, *, blocked: bool = False
    ) -> list[CrawlRequestOutcome]:
        """Lists the requests `retry_failed` would re-admit, each with its outcome.

        Reads from a crawl's committed state the requests whose latest outcome is
        `failed`, and with `blocked` also `blocked`, each with that outcome: exactly
        those `run.retry_failed` re-admits with the same option. A skip is a decision
        and is never listed.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            ClosedError: The Run is closed; nothing is sent.
        """
        warn_entry(
            "run.failed_requests", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS
        )
        params = RunFailedRequestsParams(run_id=self._host.id, blocked=blocked)
        answer = await self._host.call("run.failed_requests", params)
        return list(result_of(FailedRequests, answer).requests)

    async def retry_failed(
        self,
        *,
        blocked: bool = False,
        settings: CrawlSettings | None = None,
    ) -> int:
        """Re-admits the requests `failed_requests` lists and answers how many.

        Re-admits, in the same Run and its Datasets, every request of a crawl whose
        latest outcome is `failed`, and with `blocked` also `blocked`: exactly those
        `run.failed_requests` lists with the same option. Each keeps its ID, Label,
        Payload and depth; its attempts count on, with a fresh `crawl.retry.max`. A
        skip is a decision and is never re-admitted. It answers how many it
        re-admitted, 0 when there was nothing to retry. On a crawl this engine runs,
        the requests join its work at once, unless the crawl is already ending its
        Run: the call then waits for that end and re-admits them as on an ended Run.
        On an ended Run with requests to retry, the Run returns to `running` under
        `settings`, as a resume does, its Verdict and Completeness given anew when it
        ends, and the calling Connection serves its handlers. A Run with no writer
        that has not ended fails with `run_interrupted`: resume it first. A request
        to retry whose Label the crawl no longer declares, which a resume may drop,
        fails the call with `label_unknown`, naming that Label and those declared,
        and nothing is re-admitted: on an ended Run, against the Labels its latest
        start declared, before the Run returns to `running`. It is not sent again
        after a lost Connection, which fails it with `connection_lost`:
        `run.failed_requests` then lists the requests still to retry.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            RunInterruptedError: The Run has no live writer and has not ended.
            LabelUnknownError: A request to retry names a Label no longer declared.
            ClosedError: The Run is closed; nothing is sent.
        """
        warn_entry("run.retry_failed", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        fields: dict[str, object] = {"run_id": self._host.id, "blocked": blocked}
        if settings is not None:
            fields["settings"] = settings_on_wire(settings)
        project = self._client.project
        if project is not None:
            fields["project"] = project
        params = RunRetryFailedParams.model_validate(fields)
        answer = await self._host.call("run.retry_failed", params)
        return result_of(RetriedFailed, answer).readmitted

    def _committed_records(
        self, dataset: Dataset[R], after: int | None
    ) -> AsyncSequence[DatasetRecord[R]]:
        """`ctx.run.records`: the Records of `dataset` committed when it is sent.

        It pages `dataset.page`, whose `through` bounds every page to the
        last Record committed when the first was read, rather than following
        `run.records`, whose stream would hold the Run's order while the
        first page is asked (ADR 0103).
        """
        warn_entry("run.records", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        host = self._host
        through: int | None = None
        refused: RecordInvalidError | None = None

        async def read(position: int | None) -> _CommittedRecords[R]:
            nonlocal through, refused
            if refused is not None:
                # The Records before it were read: the read ends on it.
                raise refused
            params = DatasetPageParams(
                run_id=host.id,
                dataset=dataset.name,
                after=after if position is None else position,
                through=through,
            )
            page = result_of(_CommittedPage, await host.call("dataset.page", params))
            through = page.through
            records: list[DatasetRecord[R]] = []
            for record in page.records:
                try:
                    records.append(read_record(dataset, record))
                except RecordInvalidError as error:
                    refused = error
                    break
            return _CommittedRecords(
                records, page.next or 0, ended=refused is None and page.next is None
            )

        return self._paged(
            "run.records",
            read,
            items=lambda page: page.records,
            position=lambda page: page.next,
            last=lambda page: page.ended,
            credits=CREDITS,
            regrant=REGRANT,
        )

    def _rejected(
        self, dataset: Dataset[Any], after: int | None, *, committed: bool
    ) -> AsyncSequence[RejectedRecord]:
        """The rejected Records of `dataset`, followed, or `committed` when it is sent.

        A `committed` read, `ctx.run.rejected_records`, reads through the
        `rejected` count `dataset.list` answers first, the last entry's
        number since entries have no gaps, so it ends on a live Run (ADR
        0103).
        """
        warn_entry(
            "run.rejected_records", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS
        )
        host = self._host
        bound: int | None = None

        async def through() -> int | None:
            # Asked once, before the first page: every page reads through it.
            nonlocal bound
            if committed and bound is None:
                answer = await host.call(
                    "dataset.list", DatasetListParams(run_id=host.id)
                )
                summaries = result_of(DatasetList, answer).datasets
                rejected = (s.rejected for s in summaries if s.name == dataset.name)
                bound = next(rejected, 0)
            return bound

        async def read(position: int | None) -> RejectedRecords:
            # The first page is read past `after`, each next past the last.
            params = RunRejectedRecordsParams(
                run_id=host.id,
                dataset=dataset.name,
                after=after if position is None else position,
            )
            bound = await through()
            if bound is not None:
                # Set only when bounded: an unset field stays off the wire.
                params = params.model_copy(update={"through": bound})
            answer = await host.call("run.rejected_records", params)
            return result_of(RejectedRecords, answer)

        return self._paged(
            "run.rejected_records",
            read,
            items=lambda page: page.records,
            position=lambda page: page.next,
            last=lambda page: page.ended,
            credits=CREDITS,
            regrant=REGRANT,
        )

    async def finish(
        self, *, timeout: timedelta | NotGiven | None = not_given
    ) -> CrawlRunReport:
        """Waits for the crawl's Run to end and answers a crawl's report (ADR 0120).

        Lets a Run finish and answers its Run report: a Run its program drives ends at
        once, `finished` and `complete`; a crawl ends once its work items are done, and
        a keep-alive crawl (`crawl.keep_alive`), which waits for new work while nothing
        is left, ends once this call lets it and its work items are done. A Run
        something stopped first answers its report saying `stopped`. A Run that ended
        `failed` fails the call with the error that failed it, and a write at its end
        that failed, such as its Plan file's, with that write's error. Cancelling the
        call, or its `timeout` passing, which fails it with `deadline_exceeded`, ends
        only the wait.

        Left out, `timeout` takes the Run's `timeout.run`, by default none. The
        report is a crawl's: a Run's, with its request totals, its Records per
        Dataset, its Sinks, its sitemaps and the rules it learned; its
        Completeness is `incomplete` or `unverified` when a setting cut the crawl
        short, its reason naming the setting and the count it refused.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_experimental("run.finish")
        params = _finish_params(self._host.id, timeout)
        answer = await self._host.call(
            "run.finish", params, timeout_setting="timeout.run"
        )
        return result_of(CrawlRunReport, answer)

    async def stop(
        self, *, timeout: timedelta | NotGiven | None = not_given
    ) -> CrawlRunReport:
        """Stops the crawl's Run with a Checkpoint and answers its report, a crawl's.

        Stops a Run at its next safe point with a Checkpoint, `stopped` and `incomplete`
        as an abort by its program, resumable by its Run ID, and answers its Run report;
        an ended Run's report is answered as it is. The wait is bounded by
        `timeout.close`, or the call's `timeout`: past it, the call fails with
        `deadline_exceeded` and the Run still stops at its next safe point. A Run that
        ended `failed` fails the call as `run.finish` does.

        No attempt in flight commits, and its work items return to pending.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_experimental("run.stop")
        params = _stop_params(self._host.id, timeout)
        answer = await self._host.call(
            "run.stop", params, timeout_setting="timeout.close"
        )
        return result_of(CrawlRunReport, answer)

    async def report(self) -> CrawlRunReport:
        """Reads the crawl's Run report, a crawl's.

        Reads a Run's report: at once for a Run that ended, and at its end for a live
        one, which the read never ends. A Run another engine works is read through that
        engine, and one no engine works from its committed state, never taking its
        writer lock. A Run that has no live writer and has not ended fails with
        `run_interrupted`, whose `fix` names a start with its Run ID, and a Run ID the
        Run index does not list with `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_experimental("run.report")
        params = RunParams(run_id=self._host.id)
        return result_of(CrawlRunReport, await self._host.call("run.report", params))

    def progress(self) -> AsyncSequence[CrawlRunProgress]:
        """Follows the crawl's progress, a crawl's (ADR 0120), read with `async for`.

        Reads a Run's progress: what it does and why it waits. Without `after`, it
        answers at once; given `after`, the position of the value the reader last read,
        it waits for a change and answers the latest value, so values in between are
        coalesced. The value whose `last` is true ends the sequence: the Run ended, and
        its `lifecycle` and `stop_reason` say how, or no engine works it (`writer:
        none`) until a start with its Run ID resumes it. A Run another engine works is
        read through that engine, the owner the Run index names, and one no engine works
        from its committed state; a read never takes the Run's writer lock. A Run ID the
        Run index does not list fails with `run_not_found`.

        Each value is a crawl's: a Run's progress with its requests by state, its
        concurrency, the requests in flight, the domains it waits on and whether
        it waits idle, read as a Run's progress is.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            ClosedError: The Run is closed; nothing is sent.
            RuntimeError: No asyncio or Trio loop runs on this thread.
        """
        return self._progress(CrawlRunProgress)

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
        crawl = self._crawl
        handler = None if crawl is None else handler_of(crawl, label)
        if crawl is None or handler is None:
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
                await self._run_handler(crawl, handler, request, attempt)
            except GoodfetchError:
                raise
            except Exception as error:
                raise handler_failed(label, error) from error
            return {}
        # Only closing the Run cancels the attempt's scope.
        raise self._stopped()

    async def _run_handler(
        self,
        crawl: AsyncCrawl | Crawl,
        handler: Callable[..., object],
        request: CrawlRequestHandleParams,
        attempt: _Attempt,
    ) -> None:
        """Runs `crawl`'s async handler here, or its sync one on a worker thread."""
        crawl_request = self._request_of(crawl, request)
        lease = self._lease_of(crawl, request)
        page_id = request.page_id
        seed_attempt = SeedAttempt(
            crawl_request_id=lease.crawl_request_id, lease_epoch=lease.lease_epoch
        )
        if not isinstance(crawl, Crawl):
            page = (
                None if page_id is None else self._page_of(page_id, request.session.id)
            )
            operations = AsyncFoxhoundOperations(self._host)
            store = self._attempt_key_value_store(lease.key_value_store_attempt())
            run = AsyncAttemptRun(self.id, store, _AttemptReads(self, seed_attempt))
            context = AsyncCrawlContext(crawl_request, lease, run, operations, page)
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
        sync_run = AttemptRun(
            self.id,
            threaded.key_value_store(lease.key_value_store_attempt()),
            _ThreadedReads(threaded.loop, _AttemptReads(self, seed_attempt)),
        )
        sync_context = CrawlContext(
            crawl_request,
            lease,
            sync_run,
            FoxhoundOperations(threaded.host),
            sync_page,
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

    def _request_of(
        self, crawl: AsyncCrawl | Crawl, request: CrawlRequestHandleParams
    ) -> CrawlRequest[Any]:
        """The crawl request the handler reads, its Payload typed by its Label."""
        crawl_request = request.crawl_request
        return CrawlRequest(
            id=crawl_request.id,
            url=crawl_request.url,
            label=crawl_request.label,
            method=crawl_request.method.root,
            payload=payload_of(crawl, crawl_request.label, crawl_request.payload),
            headers=dict(crawl_request.headers or {}),
        )

    def _lease_of(
        self, crawl: AsyncCrawl | Crawl, request: CrawlRequestHandleParams
    ) -> HandlerAttempt:
        """The lease the attempt's context calls the engine with, on this Run."""
        return HandlerAttempt(
            run_id=self.id,
            crawl_request_id=request.crawl_request.id,
            lease_epoch=request.lease_epoch,
            crawl=crawl,
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
        _threaded[run] = _Threaded(
            loop, self._host, self._sessions, self._attempt_key_value_store
        )

    def seeds(self) -> list[Enqueued]:
        """Each seed's original answer in order, with no round trip (ADR 0102).

        Lists each seed's answer, in the order the crawl's start gave them, as
        the Frontier answered it when the Run began: added, with its crawl
        request's ID and its URL as parsed, or not added, with its reason
        (`duplicate`, `domain_policy`, `unsupported_scheme`, `invalid_url` or a
        cap's name). A relative seed is `invalid_url`, since nothing gives it a
        base. So a crawl that does nothing says why. Read from the Run's
        committed state, so it works on a Run no program holds.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        return self._crawl_run.seeds()

    @overload
    def add_seed(self, seed_or_url: RequestSeed | SitemapSeed) -> Enqueued: ...
    @overload
    def add_seed(
        self,
        seed_or_url: str,
        label: Label[None] | str,
        *,
        method: str | None = None,
        body: bytes | None = None,
        headers: Mapping[str, str] | None = None,
        priority: int | None = None,
        forefront: bool | None = None,
        key: str | None = None,
    ) -> Enqueued: ...
    @overload
    def add_seed(
        self,
        seed_or_url: str,
        label: Label[M],
        payload: M,
        *,
        method: str | None = None,
        body: bytes | None = None,
        headers: Mapping[str, str] | None = None,
        priority: int | None = None,
        forefront: bool | None = None,
        key: str | None = None,
    ) -> Enqueued: ...
    @overload
    def add_seed(
        self,
        seed_or_url: str,
        label: Label[JSONObject],
        payload: JSONObject,
        *,
        method: str | None = None,
        body: bytes | None = None,
        headers: Mapping[str, str] | None = None,
        priority: int | None = None,
        forefront: bool | None = None,
        key: str | None = None,
    ) -> Enqueued: ...
    def add_seed(
        self,
        seed_or_url: RequestSeed | SitemapSeed | str,
        label: Label[Any] | str | None = None,
        payload: object = None,
        *,
        method: str | None = None,
        body: bytes | None = None,
        headers: Mapping[str, str] | None = None,
        priority: int | None = None,
        forefront: bool | None = None,
        key: str | None = None,
    ) -> Enqueued:
        """Adds one seed to the running crawl (`run.add_seed`), as the async Run's.

        Adds one seed to a running crawl, `run.addSeed`, admitted and answered
        as soon as the Run state accepts it (written with the next Run-state
        batch at the default durability, or committed before answering at
        `full`), and answers as an enqueue does: added, with its crawl request's
        ID and its URL as parsed, or not added, with its reason (`duplicate`,
        `unsupported_scheme`, `invalid_url` or a cap's name). A relative seed is
        `invalid_url`. An idle `crawl.keep_alive` crawl takes it and goes on.
        Through a handler's context, naming its `attempt`, it is staged with
        that attempt: once the attempt ended it fails with `attempt_ended`. A
        Run this engine does not run as a live crawl, or a crawl already ending
        its Run, fails with `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        run = self._crawl_run
        candidate = build_seed_for_crawl(
            run._crawl,  # pyright: ignore[reportPrivateUsage]
            seed_or_url,
            label,
            payload,
            method,
            body,
            headers,
            priority,
            forefront,
            key,
        )
        try:
            return self._loop.call(lambda: run._add_seed(candidate, attempt=None))  # pyright: ignore[reportPrivateUsage]
        except LoopStopped:
            raise closed_error("run.add_seed") from None

    def add_seeds(
        self, seeds: Iterable[RequestSeed | SitemapSeed]
    ) -> Sequence[Enqueued]:
        """Streams seeds to the running crawl (`run.add_seeds`), as the async Run's.

        Takes a stream of seeds for a running crawl, `run.addSeeds`, and answers
        each as `run.add_seed` does, in order, so a million seeds never sit in
        memory. The engine grants the call 256 credits with `$/credit`, one seed
        each; the Client sends seeds in `$/seeds {id, seeds, end?}` batches,
        never more than its credits, and the engine admits each batch (staged
        with `attempt` when named, or written with the next Run-state batch at
        default durability), sends its answers in one `$/enqueued {id, answers}`
        and grants as many credits again. Once it answered the batch marked
        `end`, the call answers how many seeds were added and how many were not.
        Cancelling it keeps the seeds already answered. A batch beyond the
        credits granted, or a seed that is no seed, fails the call with
        `invalid_params`; a Run this engine does not run as a live crawl, or a
        crawl already ending its Run, fails with `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        run = self._crawl_run

        async def opening() -> AsyncSequence[Enqueued]:
            return run._add_seeds(seeds, attempt=None)  # pyright: ignore[reportPrivateUsage]

        return _on_thread(self._loop, "run.add_seeds", opening)

    def records(
        self, dataset: Dataset[R], *, after: int | None = None
    ) -> Sequence[DatasetRecord[R]]:
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

        Each Record's `value` is the Dataset's own type, parsed by its model
        with each field the model does not declare kept in `model_extra`; a
        Record the model refuses ends the read with `RecordInvalidError`
        naming its `number`, past which `after` resumes, once the Records
        before it are read. `through` and `wall` are not taken yet.

        Experimental: outside semver until it is promoted (ADR 0030).

        Any thread may read it. Leaving a `with` block, `close()` or
        releasing it cancel its read at once.

        Raises:
            ClosedError: The Run or its Connection is closed; nothing is sent.
        """
        warn_entry("run.records", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        run = self._crawl_run

        async def reading() -> AsyncSequence[DatasetRecord[R]]:
            return run.records(dataset, after=after)

        return _on_thread(self._loop, "run.records", reading)

    def rejected_records(
        self, dataset: Dataset[Any], *, after: int | None = None
    ) -> Sequence[RejectedRecord]:
        """The rejected Records of `dataset`, read with `for`, as the async Run's.

        Reads one Dataset's rejection table from a crawl's committed state:
        each Record rejected rather than committed, numbered from 1 in commit
        order, with the Record as JSON, the reason, each issue's path, the
        Hook when one rejected it, and the request's ID and URL. It starts at
        the first entry, or after `after`, the number a reader last handled.
        While the Run is live and nothing follows, it waits for the next
        commit; a page says `ended` once the Run has ended, or has no live
        writer, and no entry follows. With `through`, such as the `rejected`
        count `dataset.list` answers, it reads no entry numbered past it and
        never waits, a live Run's included: a page says `ended` once it
        reaches that number or has read every committed entry. A reader never
        slows the Run. A Dataset the crawl does not declare fails with
        `dataset_unknown`.

        Each entry's `data` is the Record's JSON as it was pushed, never
        parsed. The read goes page after page, each next page once the
        program has read on, and ends once a page says the table is read to
        its end. `through` is not taken here: a handler's `ctx.run` read
        bounds itself.

        Experimental: outside semver until it is promoted (ADR 0030).

        Any thread may read it. Leaving a `with` block, `close()` or
        releasing it cancel its read at once.

        Raises:
            ClosedError: The Run or its Connection is closed; nothing is sent.
        """
        warn_entry(
            "run.rejected_records", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS
        )
        run = self._crawl_run

        async def reading() -> AsyncSequence[RejectedRecord]:
            return run.rejected_records(dataset, after=after)

        return _on_thread(self._loop, "run.rejected_records", reading)

    def export(
        self,
        dataset: Dataset[Any],
        *,
        format: ExportFormat | str,
        compression: ExportCompression | str | None = None,
        wall: bool | None = None,
        settings: CrawlSettings | None = None,
    ) -> Export:
        """Exports `dataset`'s committed Records into an `Export`, as the async Run's.

        Writes an Export of one Dataset of a crawl's Run: the Records it
        committed when the call is made, in number order, in the format named,
        as its `format` value says: a table (`csv`, `xlsx`, `html_table`,
        `markdown`, and typed, `parquet`, `avro` and `arrow_ipc`) has one
        column per field seen in any of its Records, in first-seen order, and
        one row per Record. Every text Export is UTF-8 with no BOM unless
        `export.csv.bom` puts one on CSV, characters written as themselves (a
        character XML 1.0 cannot hold as U+FFFD in XML and feeds) and each
        value as it was committed: a table cell holds a string as its text, a
        number, `true` or `false` as JSON writes it, nothing for `null` or a
        field its Record lacks, and an object or array as its JSON text, or, in
        CSV under `export.csv.nested: flatten`, an object's fields in dotted
        columns. A Record whose page was read past a Wall carries its Wall
        marker, `{kind, vendor?}` as its Provenance holds it, as a `#wall`
        field in every format but the feeds, where it is a category of the
        scheme `urn:goodfetch:wall` naming its kind; `wall` set to `true`
        writes only those Records, `false` only the others. `compression`
        `gzip` or `zstd` compresses the whole file. Its `settings` and
        `project` are the run layer and the project configuration its
        `export.*` settings resolve under; a value the format cannot take fails
        with `setting_invalid`. Answers the Export, with its Record count, its
        last Record's number, its size, its SHA-256 and the run epoch of the
        Run's state it was written from, only once its file is whole in the
        Run's folder, where `export.save_as` and `export.stream` read it.
        Records committed after the call starts are not in it. A write the
        engine's storage refuses fails with `file_write_failed`, side `engine`,
        leaving no unfinished file, as does a call while that storage is at the
        `critical` level, before any byte is written. A Dataset the crawl does
        not declare fails with `dataset_unknown`; an `xlsx` sheet past Excel's
        limits, with `sheet_capacity_exceeded`.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            DatasetUnknownError: `dataset` is not declared by this crawl.
            SettingInvalidError: A setting is not valid for the chosen format.
            SheetCapacityExceededError: An `xlsx` sheet passes Excel's limits.
            FileWriteFailedError: The engine's storage refused the write.
            ClosedError: The Run or its Connection is closed; nothing is sent.
        """
        run = self._crawl_run
        try:
            async_export = self._loop.call(
                lambda: run.export(
                    dataset,
                    format=format,
                    compression=compression,
                    wall=wall,
                    settings=settings,
                )
            )
        except LoopStopped:
            raise closed_error("run.export") from None
        return Export(self._loop, async_export)

    def outcomes(
        self,
        *,
        kinds: Iterable[OutcomeKind] | None = None,
        labels: Iterable[str | Label[Any]] | None = None,
        codes: Iterable[str] | None = None,
        after: int | Literal["now"] | None = None,
    ) -> Sequence[CrawlRequestOutcome]:
        """A log of endings, not current state; retries supersede by `request.id`.

        Reads a crawl's outcomes from its committed state, in commit order: from the
        first, after a position, or with `after: "now"` from the next commit. Its
        filters, `kinds`, `labels` and `codes` (an Error code any of the request's
        failed attempts failed with), run in the engine. While the Run is live and
        nothing new matches, it waits for the next commit; a page says `ended` once
        the Run has ended, or has no live writer, and nothing more matches. A reader
        never slows the Run. A request `run.retry_failed` re-admits ends again, and
        its later outcome supersedes the earlier one by `request.id`.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            ClosedError: The Run or its Connection is closed; nothing is sent.
        """
        warn_entry("run.outcomes", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        run = self._crawl_run
        kind_list = None if kinds is None else list(kinds)
        label_list = None if labels is None else list(labels)
        code_list = None if codes is None else list(codes)

        async def reading() -> AsyncSequence[CrawlRequestOutcome]:
            return run.outcomes(
                kinds=kind_list, labels=label_list, codes=code_list, after=after
            )

        return _on_thread(self._loop, "run.outcomes", reading)

    def failed_requests(self, *, blocked: bool = False) -> list[CrawlRequestOutcome]:
        """Lists the requests `retry_failed` would re-admit, each with its outcome.

        Reads from a crawl's committed state the requests whose latest outcome is
        `failed`, and with `blocked` also `blocked`, each with that outcome: exactly
        those `run.retry_failed` re-admits with the same option. A skip is a decision
        and is never listed.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            ClosedError: The Run or its Connection is closed; nothing is sent.
        """
        run = self._crawl_run
        try:
            return self._loop.call(lambda: run.failed_requests(blocked=blocked))
        except LoopStopped:
            raise closed_error("run.failed_requests") from None

    def retry_failed(
        self,
        *,
        blocked: bool = False,
        settings: CrawlSettings | None = None,
    ) -> int:
        """Re-admits the requests `failed_requests` lists and answers how many.

        Re-admits, in the same Run and its Datasets, every request of a crawl whose
        latest outcome is `failed`, and with `blocked` also `blocked`: exactly those
        `run.failed_requests` lists with the same option. Each keeps its ID, Label,
        Payload and depth; its attempts count on, with a fresh `crawl.retry.max`. A
        skip is a decision and is never re-admitted. It answers how many it
        re-admitted, 0 when there was nothing to retry. On a crawl this engine runs,
        the requests join its work at once, unless the crawl is already ending its
        Run: the call then waits for that end and re-admits them as on an ended Run.
        On an ended Run with requests to retry, the Run returns to `running` under
        `settings`, as a resume does, its Verdict and Completeness given anew when it
        ends, and the calling Connection serves its handlers. A Run with no writer
        that has not ended fails with `run_interrupted`: resume it first. A request
        to retry whose Label the crawl no longer declares, which a resume may drop,
        fails the call with `label_unknown`, naming that Label and those declared,
        and nothing is re-admitted: on an ended Run, against the Labels its latest
        start declared, before the Run returns to `running`. It is not sent again
        after a lost Connection, which fails it with `connection_lost`:
        `run.failed_requests` then lists the requests still to retry.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            RunInterruptedError: The Run has no live writer and has not ended.
            LabelUnknownError: A request to retry names a Label no longer declared.
            ClosedError: The Run or its Connection is closed; nothing is sent.
        """
        run = self._crawl_run
        try:
            return self._loop.call(
                lambda: run.retry_failed(blocked=blocked, settings=settings)
            )
        except LoopStopped:
            raise closed_error("run.retry_failed") from None

    def finish(
        self, *, timeout: timedelta | NotGiven | None = not_given
    ) -> CrawlRunReport:
        """Waits for the crawl's Run to end and answers its report, as the async Run's.

        Lets a Run finish and answers its Run report: a Run its program drives ends at
        once, `finished` and `complete`; a crawl ends once its work items are done, and
        a keep-alive crawl (`crawl.keep_alive`), which waits for new work while nothing
        is left, ends once this call lets it and its work items are done. A Run
        something stopped first answers its report saying `stopped`. A Run that ended
        `failed` fails the call with the error that failed it, and a write at its end
        that failed, such as its Plan file's, with that write's error. Cancelling the
        call, or its `timeout` passing, which fails it with `deadline_exceeded`, ends
        only the wait.

        Left out, `timeout` takes the Run's `timeout.run`, by default none. The
        report is a crawl's: a Run's, with its request totals, its Records per
        Dataset, its Sinks, its sitemaps and the rules it learned; its
        Completeness is `incomplete` or `unverified` when a setting cut the crawl
        short, its reason naming the setting and the count it refused.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_experimental("run.finish")
        params = _finish_params(self._host.id, timeout)
        answer = self._host.call("run.finish", params, timeout_setting="timeout.run")
        return result_of(CrawlRunReport, answer)

    def stop(
        self, *, timeout: timedelta | NotGiven | None = not_given
    ) -> CrawlRunReport:
        """Stops the crawl's Run and answers its report, as the async Run's.

        Stops a Run at its next safe point with a Checkpoint, `stopped` and `incomplete`
        as an abort by its program, resumable by its Run ID, and answers its Run report;
        an ended Run's report is answered as it is. The wait is bounded by
        `timeout.close`, or the call's `timeout`: past it, the call fails with
        `deadline_exceeded` and the Run still stops at its next safe point. A Run that
        ended `failed` fails the call as `run.finish` does.

        No attempt in flight commits, and its work items return to pending.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_experimental("run.stop")
        params = _stop_params(self._host.id, timeout)
        answer = self._host.call("run.stop", params, timeout_setting="timeout.close")
        return result_of(CrawlRunReport, answer)

    def report(self) -> CrawlRunReport:
        """Reads the crawl's Run report, as the async Run's.

        Reads a Run's report: at once for a Run that ended, and at its end for a live
        one, which the read never ends. A Run another engine works is read through that
        engine, and one no engine works from its committed state, never taking its
        writer lock. A Run that has no live writer and has not ended fails with
        `run_interrupted`, whose `fix` names a start with its Run ID, and a Run ID the
        Run index does not list with `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_experimental("run.report")
        params = RunParams(run_id=self._host.id)
        return result_of(CrawlRunReport, self._host.call("run.report", params))

    def progress(self) -> Sequence[CrawlRunProgress]:
        """Follows the crawl's progress, read with `for`, as the async Run's.

        Reads a Run's progress: what it does and why it waits. Without `after`, it
        answers at once; given `after`, the position of the value the reader last read,
        it waits for a change and answers the latest value, so values in between are
        coalesced. The value whose `last` is true ends the sequence: the Run ended, and
        its `lifecycle` and `stop_reason` say how, or no engine works it (`writer:
        none`) until a start with its Run ID resumes it. A Run another engine works is
        read through that engine, the owner the Run index names, and one no engine works
        from its committed state; a read never takes the Run's writer lock. A Run ID the
        Run index does not list fails with `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            ClosedError: The Run is closed; nothing is sent.
        """
        # Its async twin reads each value as a crawl's progress.
        return cast("Sequence[CrawlRunProgress]", super().progress())


class _CommittedPage(BaseModel):
    """One page of `dataset.page`, its Records ready to read as their Dataset's type."""

    model_config = ConfigDict(extra="allow")
    next: int | None = None
    """The next page's `after`; absent once no Record follows."""
    records: list[DatasetRecord[Any]]
    """The Records, in number order."""
    through: int
    """The last Record committed when the first page was read."""


@dataclass(frozen=True)
class _CommittedRecords(Generic[R]):
    """One page of a handler's committed read, its Records of their Dataset's type."""

    records: list[DatasetRecord[R]]
    """The Records its Dataset's model read, up to one it refused."""
    next: int
    """The next page's `after`."""
    ended: bool
    """Whether no Record follows and none was refused."""


@final
class _AttemptReads:
    """What an `AsyncAttemptRun` reads and seeds its Run through."""

    def __init__(self, run: AsyncCrawlRun, attempt: SeedAttempt) -> None:
        """Takes the crawl's Run and attempt lease."""
        self._run = run
        self._attempt = attempt

    def records(
        self, dataset: Dataset[R], after: int | None
    ) -> AsyncSequence[DatasetRecord[R]]:
        """The Records of `dataset` committed when it is sent, past `after`."""
        return self._run._committed_records(dataset, after)  # pyright: ignore[reportPrivateUsage]

    def rejected_records(
        self, dataset: Dataset[Any], after: int | None
    ) -> AsyncSequence[RejectedRecord]:
        """The rejected Records of `dataset` committed when it is sent, past `after`."""
        return self._run._rejected(dataset, after, committed=True)  # pyright: ignore[reportPrivateUsage]

    async def add_seed(self, candidate: RequestSeed | SitemapSeed) -> Enqueued:
        """Adds one seed staged with this attempt."""
        return await self._run._add_seed(candidate, attempt=self._attempt)  # pyright: ignore[reportPrivateUsage]

    def add_seeds(
        self,
        seeds: Iterable[RequestSeed | SitemapSeed]
        | AsyncIterable[RequestSeed | SitemapSeed],
    ) -> AsyncSequence[Enqueued]:
        """Streams seeds staged with this attempt."""
        return self._run._add_seeds(seeds, attempt=self._attempt)  # pyright: ignore[reportPrivateUsage]

    def build_seed(
        self,
        seed_or_url: RequestSeed | SitemapSeed | str,
        label: Label[Any] | str | None,
        payload: object,
        method: str | None,
        body: bytes | None,
        headers: Mapping[str, str] | None,
        priority: int | None,
        forefront: bool | None,
        key: str | None,
    ) -> RequestSeed | SitemapSeed:
        """Builds and validates one seed against the Run's crawl."""
        return build_seed_for_crawl(
            self._run._crawl,  # pyright: ignore[reportPrivateUsage]
            seed_or_url,
            label,
            payload,
            method,
            body,
            headers,
            priority,
            forefront,
            key,
        )


@final
class _ThreadedReads:
    """What an `AttemptRun` reads and seeds its Run through, on the loop."""

    def __init__(self, loop: PrivateLoop, reads: _AttemptReads) -> None:
        """Takes the Connection's private loop and the async reads it drives there."""
        self._loop = loop
        self._reads = reads

    def records(
        self, dataset: Dataset[R], after: int | None
    ) -> Sequence[DatasetRecord[R]]:
        """The Records of `dataset` committed when it is sent, past `after`."""
        reads = self._reads

        async def reading() -> AsyncSequence[DatasetRecord[R]]:
            return reads.records(dataset, after)

        return _on_thread(self._loop, "run.records", reading)

    def rejected_records(
        self, dataset: Dataset[Any], after: int | None
    ) -> Sequence[RejectedRecord]:
        """The rejected Records of `dataset` committed when it is sent, past `after`."""
        reads = self._reads

        async def reading() -> AsyncSequence[RejectedRecord]:
            return reads.rejected_records(dataset, after)

        return _on_thread(self._loop, "run.rejected_records", reading)

    def add_seed(self, candidate: RequestSeed | SitemapSeed) -> Enqueued:
        """Adds one seed staged with this attempt, on the loop."""
        reads = self._reads
        try:
            return self._loop.call(lambda: reads.add_seed(candidate))
        except LoopStopped:
            raise closed_error("run.add_seed") from None

    def add_seeds(
        self, seeds: Iterable[RequestSeed | SitemapSeed]
    ) -> Sequence[Enqueued]:
        """Streams seeds staged with this attempt, on the loop."""
        reads = self._reads

        async def opening() -> AsyncSequence[Enqueued]:
            return reads.add_seeds(seeds)

        return _on_thread(self._loop, "run.add_seeds", opening)

    def build_seed(
        self,
        seed_or_url: RequestSeed | SitemapSeed | str,
        label: Label[Any] | str | None,
        payload: object,
        method: str | None,
        body: bytes | None,
        headers: Mapping[str, str] | None,
        priority: int | None,
        forefront: bool | None,
        key: str | None,
    ) -> RequestSeed | SitemapSeed:
        """Builds and validates one seed against the Run's crawl."""
        return self._reads.build_seed(
            seed_or_url,
            label,
            payload,
            method,
            body,
            headers,
            priority,
            forefront,
            key,
        )


def _on_thread(
    loop: PrivateLoop,
    operation: str,
    reading: Callable[[], Awaitable[AsyncSequence[_Item]]],
) -> Sequence[_Item]:
    """The async read `reading` opens on `loop`, read from the calling thread.

    Raises:
        ClosedError: The Connection is closed; nothing is sent.
    """
    try:
        sequence = loop.call(reading)
    except LoopStopped:
        raise closed_error(operation) from None
    return Sequence(loop, sequence, lambda: closed_error(operation))


@dataclass(frozen=True)
class _Threaded:
    """What a sync crawl's handlers reach its Run through, from the loop.

    It holds the `CrawlRun`'s host and the Sessions it keeps by ID, not the
    `CrawlRun`, which holds the async Run that keys it in `_threaded`.
    """

    loop: PrivateLoop
    """The Connection's private loop, which its handlers' reads run on."""
    host: HandleHost[RunId]
    """The sync host its handlers call through."""
    sessions: dict[str, tuple[HandleHost[SessionId], Session]]
    """The Sessions the `CrawlRun` keeps by ID, which its pages share."""
    key_value_store: Callable[[KeyValueStoreAttempt], KeyValueStore]
    """Opens the Run's Key-value store bound to one of its handlers' attempts."""

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
        crawl: AsyncCrawl | CodeFreeCrawl,
        *,
        seeds: Iterable[RequestSeed | SitemapSeed],
        run_id: str | None,
        tags: Iterable[str] | None,
        limits: Limits | None,
        settings: CrawlSettings | None,
        detached: bool,
    ) -> AsyncCrawlRun:
        """Starts `crawl` from `seeds`: `AsyncFoxhound.start_crawl`.

        Raises:
            LabelUnhandledError: A Label has no handler; nothing is sent.
            LabelUnknownError: A seed names a Label `crawl` does not declare.
            ClosedError: The Connection is closed; nothing is sent.
        """
        served = (
            None if isinstance(crawl, CrawlDeclaration | CrawlExtraction) else crawl
        )
        return await self._start(
            crawl,
            functools.partial(self._opened, served, detached),
            seeds=seeds,
            run_id=run_id,
            tags=tags,
            limits=limits,
            settings=settings,
            detached=detached,
        )

    async def start_threaded(
        self,
        loop: PrivateLoop,
        crawl: Crawl | CodeFreeCrawl,
        *,
        seeds: Iterable[RequestSeed | SitemapSeed],
        run_id: str | None,
        tags: Iterable[str] | None,
        limits: Limits | None,
        settings: CrawlSettings | None,
        detached: bool,
    ) -> CrawlRun:
        """Starts the sync `crawl` from `seeds` on `loop`: `Foxhound.start_crawl`.

        Raises:
            LabelUnhandledError: A Label has no handler; nothing is sent.
            LabelUnknownError: A seed names a Label `crawl` does not declare.
            ClosedError: The Connection is closed; nothing is sent.
        """
        threaded: list[CrawlRun] = []
        served = (
            None if isinstance(crawl, CrawlDeclaration | CrawlExtraction) else crawl
        )

        def opened(started: RunStarted, seeds: list[Enqueued]) -> AsyncCrawlRun:
            run = self._opened(served, detached, started, seeds)
            threaded.append(CrawlRun(loop, run))
            return run

        await self._start(
            crawl,
            opened,
            seeds=seeds,
            run_id=run_id,
            tags=tags,
            limits=limits,
            settings=settings,
            detached=detached,
        )
        return threaded[0]

    async def _start(
        self,
        crawl: AsyncCrawl | Crawl | CodeFreeCrawl,
        opened: Callable[[RunStarted, list[Enqueued]], AsyncCrawlRun],
        *,
        seeds: Iterable[RequestSeed | SitemapSeed],
        run_id: str | None,
        tags: Iterable[str] | None,
        limits: Limits | None,
        settings: CrawlSettings | None,
        detached: bool,
    ) -> AsyncCrawlRun:
        """Sends `crawl.start`, and `opened` builds the Run it answers.

        Raises:
            SettingInvalidError: A `limits` amount the SDK cannot turn
                exactly; nothing is sent.
        """
        sent_seeds = list(seeds)
        fields = _crawl_shape(crawl, sent_seeds)
        fields["seeds"] = [_wire(sent) for sent in sent_seeds]
        if run_id is not None:
            fields["run_id"] = run_id
        if tags is not None:
            fields["tags"] = list(tags)
        run_limits = limit_list(limits)
        if run_limits:
            fields["limits"] = run_limits
        if settings is not None:
            fields["settings"] = settings_on_wire(settings)
        # The start carries the project file `connect()` read (ADR 0083).
        project = self._connection.project
        if project is not None:
            fields["project"] = project
        if detached:
            fields["detached"] = True
        params = CrawlStartParams.model_construct(None, **fields)
        operations = AsyncFoxhoundOperations(self._connection)
        starting = anyio.Event()
        self._starting.add(starting)
        started_seeds: list[Enqueued] = []

        async def start_crawl() -> RunStarted:
            crawl_started = await operations.crawl_start(params)
            if crawl_started.seeds:
                started_seeds.extend(crawl_started.seeds)
            return RunStarted(
                run_id=crawl_started.run_id,
                lifecycle=crawl_started.lifecycle,
                started_as=crawl_started.started_as,
                needs=crawl_started.needs,
            )

        try:
            return await self._connection.start_run(
                "crawl.start",
                start_crawl,
                lambda started: opened(started, started_seeds),
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

    def _opened(
        self,
        crawl: AsyncCrawl | Crawl | None,
        detached: bool,
        started: RunStarted,
        seeds: list[Enqueued],
    ) -> AsyncCrawlRun:
        return AsyncCrawlRun(
            self._connection, started, crawl, self, seeds=seeds, detached=detached
        )

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


def _finish_params(
    run_id: RunId, timeout: timedelta | NotGiven | None
) -> RunFinishParams:
    """`run.finish`'s params, its `timeout` off the wire when not given."""
    params = RunFinishParams(run_id=run_id)
    if not isinstance(timeout, NotGiven):
        params.timeout = timeout_of(timeout, "timeout.run")
    return params


def _stop_params(run_id: RunId, timeout: timedelta | NotGiven | None) -> RunStopParams:
    """`run.stop`'s params, its `timeout` off the wire when not given."""
    params = RunStopParams(run_id=run_id)
    if not isinstance(timeout, NotGiven):
        params.timeout = timeout_of(timeout, "timeout.close")
    return params


def _record_of(method: str, params: object) -> DatasetRecord[Any] | None:
    """The Record a `$/record` notification carries, unparsed; none for another."""
    if method != RECORD or not isinstance(params, dict):
        return None
    record = cast("dict[str, object]", params).get("record")
    return None if record is None else result_of(DatasetRecord[Any], record)


def _crawl_shape(
    crawl: AsyncCrawl | Crawl | CodeFreeCrawl,
    seeds: list[RequestSeed | SitemapSeed],
) -> dict[str, object]:
    """What `crawl.start` takes for `crawl`: its Labels and Datasets, or itself.

    A code-free crawl goes as its `declaration` or its `extraction`.

    Raises:
        LabelUnhandledError: A Label of the program's crawl has no handler.
        LabelUnknownError: A seed names a Label the program's crawl does not
            declare.
    """
    if isinstance(crawl, CrawlDeclaration):
        return {"declaration": _wire(crawl)}
    if isinstance(crawl, CrawlExtraction):
        return {"extraction": _wire(crawl)}
    check_startable(crawl, seeds)
    labels, datasets = crawl_declaration(crawl)
    fields: dict[str, object] = {
        "labels": {name: _wire(label) for name, label in labels.items()}
    }
    if datasets:
        fields["datasets"] = [_wire(dataset) for dataset in datasets]
    return fields


def _wire(model: BaseModel) -> dict[str, object]:
    """`model` as the wire spells it, a field left unset left out."""
    return model.model_dump(
        mode="json", by_alias=True, exclude_unset=True, warnings=False
    )
