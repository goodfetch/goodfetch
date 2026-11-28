"""A read-only view of a crawl's Run another program started (ADRs 0102, 0103).

`AsyncFoxhound.open_crawl_run()` and the sync `Foxhound.open_crawl_run()` open
an `AsyncCrawlRunView` / `CrawlRunView` by the Run's ID with no round trip and
no writer lock. Closing it frees only the view: the Run goes on.
TypeScript's `crawl-run-view.ts` and Rust's `crawl/view.rs` are its twins.
"""

from __future__ import annotations

import functools
from typing import TYPE_CHECKING, Any, Literal, TypeVar, cast

from goodfetch_collie_client._followed import PagedReading
from goodfetch_collie_client.caller import result_of
from goodfetch_collie_client.connection import closed_error
from goodfetch_collie_client.generated.models import (
    RunId,
    RunParams,
    RunProgressParams,
)
from goodfetch_collie_client.listeners import CREDITS, REGRANT, Order, spawner
from goodfetch_collie_client.markers import warn_entry
from goodfetch_collie_client.portal import LoopStopped
from goodfetch_collie_client.run_view import AsyncRunView, RunView
from goodfetch_collie_client.sequence import AsyncSequence, Reading, Sequence
from goodfetch_collie_client.settings import settings_on_wire

from goodfetch_foxhound.crawl import Dataset, DatasetRecord, Label, read_record
from goodfetch_foxhound.crawl_run import RECORD
from goodfetch_foxhound.export import AsyncExport, Export
from goodfetch_foxhound.generated.models import (
    CrawlRequestOutcome,
    CrawlRequestOutcomes,
    CrawlRunProgress,
    CrawlRunReport,
    ExportCompression,
    ExportFormat,
    FailedRequests,
    Now,
    OutcomesAfter,
    OutcomesAfter1,
    RejectedRecord,
    RejectedRecords,
    RunExportParams,
    RunFailedRequestsParams,
    RunOutcomesParams,
    RunRejectedRecordsParams,
)
from goodfetch_foxhound.generated.models import (
    Export as WireExport,
)
from goodfetch_foxhound.generated.operations import (
    DEPRECATED_OPERATIONS,
    EXPERIMENTAL_OPERATIONS,
)

if TYPE_CHECKING:
    from collections.abc import Callable, Iterable, Mapping

    from goodfetch_collie_client.connection import ClientConnection
    from goodfetch_collie_client.portal import PrivateLoop

    from goodfetch_foxhound.generated.open_values import OutcomeKind
    from goodfetch_foxhound.generated.settings import CrawlSettings

__all__ = ["AsyncCrawlRunView", "CrawlRunView"]

R = TypeVar("R")
_Item = TypeVar("_Item")


def _record_of(method: str, notified: object) -> DatasetRecord[Any] | None:
    if method != RECORD or not isinstance(notified, dict):
        return None
    record = cast("dict[str, object]", notified).get("record")
    return None if record is None else result_of(DatasetRecord[Any], record)


class AsyncCrawlRunView(AsyncRunView):
    """A read-only view of a crawl's Run opened by `AsyncFoxhound.open_crawl_run`."""

    def __init__(self, connection: ClientConnection, run_id: str) -> None:
        """Takes the Connection and the Run ID `open_crawl_run()` was given."""
        super().__init__(connection, run_id)

    def _read(
        self,
        operation: str,
        params: Mapping[str, object],
        item_of: Callable[[str, object], _Item | None],
    ) -> AsyncSequence[_Item]:
        self._ensure_open(operation)
        reading = Reading(self._connection, operation, params, item_of, Order())
        spawner()(reading.run)
        return AsyncSequence(reading)

    def progress(self) -> AsyncSequence[CrawlRunProgress]:
        """Follows the crawl Run's progress (`run.progress`, ADRs 0102, 0120).

        Reads a Run's progress: what it does and why it waits. Without `after`,
        it answers at once; given `after`, the position of the value the reader
        last read, it waits for a change and answers the latest value, so values
        in between are coalesced. The value whose `last` is true ends the
        sequence: the Run ended, and its `lifecycle` and `stop_reason` say how,
        or no engine works it (`writer: none`) until a start with its Run ID
        resumes it. A Run another engine works is read through that engine, the
        owner the Run index names, and one no engine works from its committed
        state; a read never takes the Run's writer lock. A Run ID the Run index
        does not list fails with `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("run.progress", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        self._ensure_open("run.progress")
        run_id = RunId(self._id)

        async def read(after: int | None) -> CrawlRunProgress:
            self._ensure_open("run.progress")
            params = RunProgressParams(run_id=run_id)
            if after is not None:
                params.after = after
            return result_of(
                CrawlRunProgress, await self._connection.call("run.progress", params)
            )

        reading = PagedReading(
            read,
            items=lambda value: (value,),
            position=lambda value: value.position,
            last=lambda value: value.last,
            credits=1,
            regrant=1,
        )
        spawner()(reading.run)
        return AsyncSequence(reading)

    async def report(self) -> CrawlRunReport:
        """Reads the crawl Run's report (`run.report`, ADRs 0102, 0120).

        Reads a Run's report: at once for a Run that ended, and at its end for
        a live one, which the read never ends. A Run another engine works is
        read through that engine, and one no engine works from its committed
        state, never taking its writer lock. A Run that has no live writer and
        has not ended fails with `run_interrupted`, whose `fix` names a start
        with its Run ID, and a Run ID the Run index does not list with
        `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("run.report", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        self._ensure_open("run.report")
        raw = await self._connection.call(
            "run.report", RunParams(run_id=RunId(self._id))
        )
        return result_of(CrawlRunReport, raw)

    def records(
        self, dataset: Dataset[R], *, after: int | None = None
    ) -> AsyncSequence[DatasetRecord[R]]:
        """Reads the committed Records of `dataset` in number order (`run.records`).

        Reads the committed Records of one Dataset of a crawl's Run in number
        order, from the first, or after the number `after`, which resumes a
        reader from the last Record it handled. Each Record goes in a
        `$/record` notification with its number, its value, its request's ID
        and URL and when it was committed, one per credit granted with
        `$/credit`. While the Run is live it follows the Run, sending each
        Record once it commits; once the Run has ended, or has no live writer,
        and every Record is sent, it answers how many it sent and the last
        one's number. With `through`, such as the number `dataset.page`
        answers as the last committed, it reads no Record numbered past it and
        answers once it sent every Record through it, a live Run's included.
        Records are read from the Run's committed state on a connection of
        their own, so a reader that grants no credit never slows the crawl.
        With `wall` set to `true` it reads only the Records whose page was
        read past a Wall, which their Provenance marks, and with `false` only
        the others (ADR 0052). A Dataset the crawl does not declare fails with
        `dataset_unknown`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("run.records", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        params: dict[str, object] = {"run_id": self._id, "dataset": dataset.name}
        if after is not None:
            params["after"] = after

        def item_of(method: str, notified: object) -> DatasetRecord[R] | None:
            record = _record_of(method, notified)
            return None if record is None else read_record(dataset, record)

        return self._read("run.records", params, item_of)

    def rejected_records(
        self, dataset: Dataset[Any], *, after: int | None = None
    ) -> AsyncSequence[RejectedRecord]:
        """Reads `dataset`'s rejection table from committed state.

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

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry(
            "run.rejected_records", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS
        )
        self._ensure_open("run.rejected_records")
        run_id = RunId(self._id)

        async def read(position: int | None) -> RejectedRecords:
            self._ensure_open("run.rejected_records")
            cursor = after if position is None else position
            params = RunRejectedRecordsParams(
                run_id=run_id,
                dataset=dataset.name,
                after=cursor,
            )
            answer = await self._connection.call("run.rejected_records", params)
            return result_of(RejectedRecords, answer)

        reading = PagedReading(
            read,
            items=lambda page: page.records,
            position=lambda page: page.next,
            last=lambda page: page.ended,
            credits=CREDITS,
            regrant=REGRANT,
        )
        spawner()(reading.run)
        return AsyncSequence(reading)

    async def export(
        self,
        dataset: Dataset[Any],
        *,
        format: ExportFormat | str,
        compression: ExportCompression | str | None = None,
        wall: bool | None = None,
        settings: CrawlSettings | None = None,
    ) -> AsyncExport:
        """Exports `dataset`'s committed Records into an `AsyncExport` (`run.export`).

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
        """
        warn_entry("run.export", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        self._ensure_open("run.export")
        fields: dict[str, object] = {
            "run_id": RunId(self._id),
            "dataset": dataset.name,
            "format": ExportFormat(format),
        }
        if compression is not None:
            fields["compression"] = ExportCompression(compression)
        if wall is not None:
            fields["wall"] = wall
        if settings is not None:
            fields["settings"] = settings_on_wire(settings)
        project = self._connection.project
        if project is not None:
            fields["project"] = project
        params = RunExportParams.model_validate(fields)
        answer = await self._connection.call("run.export", params)
        descriptor = result_of(WireExport, answer)
        return AsyncExport(self._connection, descriptor, self._read)

    def outcomes(
        self,
        *,
        kinds: Iterable[OutcomeKind] | None = None,
        labels: Iterable[str | Label[Any]] | None = None,
        codes: Iterable[str] | None = None,
        after: int | Literal["now"] | None = None,
    ) -> AsyncSequence[CrawlRequestOutcome]:
        """Reads the crawl's request outcomes in commit order (`run.outcomes`).

        Reads a crawl's outcomes from its committed state, in commit order: from
        the first, after a position, or with `after: "now"` from the next commit.
        Its filters, `kinds`, `labels` and `codes` (an Error code any of the
        request's failed attempts failed with), run in the engine. While the Run
        is live and nothing new matches, it waits for the next commit; a page
        says `ended` once the Run has ended, or has no live writer, and nothing
        more matches. A reader never slows the Run. A request `run.retry_failed`
        re-admits ends again, and its later outcome supersedes the earlier one
        by `request.id`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("run.outcomes", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        self._ensure_open("run.outcomes")
        run_id = RunId(self._id)
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
            self._ensure_open("run.outcomes")
            cursor = (
                initial_after
                if position is None
                else OutcomesAfter(OutcomesAfter1(position))
            )
            params = RunOutcomesParams(
                run_id=run_id,
                kinds=kind_list,
                labels=label_list,
                codes=code_list,
                after=cursor,
            )
            answer = await self._connection.call("run.outcomes", params)
            return result_of(CrawlRequestOutcomes, answer)

        reading = PagedReading(
            read,
            items=lambda page: page.outcomes,
            position=lambda page: page.next,
            last=lambda page: page.ended,
            credits=CREDITS,
            regrant=REGRANT,
        )
        spawner()(reading.run)
        return AsyncSequence(reading)

    async def failed_requests(
        self, *, blocked: bool = False
    ) -> list[CrawlRequestOutcome]:
        """Lists the requests whose latest outcome is `failed` (`run.failed_requests`).

        Reads from a crawl's committed state the requests whose latest outcome
        is `failed`, and with `blocked` also `blocked`, each with that outcome:
        exactly those `run.retry_failed` re-admits with the same option. A skip
        is a decision and is never listed.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry(
            "run.failed_requests", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS
        )
        self._ensure_open("run.failed_requests")
        params = RunFailedRequestsParams(run_id=RunId(self._id), blocked=blocked)
        answer = await self._connection.call("run.failed_requests", params)
        return list(result_of(FailedRequests, answer).requests)


class CrawlRunView(RunView):
    """A read-only view of a crawl's Run opened by `Foxhound.open_crawl_run`."""

    def __init__(self, loop: PrivateLoop, view: AsyncCrawlRunView) -> None:
        """Takes the Connection's private loop and the view it opened there."""
        super().__init__(loop, view)
        self._crawl_view = view

    def progress(self) -> Sequence[CrawlRunProgress]:
        """Follows the crawl Run's progress, as `AsyncCrawlRunView.progress`.

        Reads a Run's progress: what it does and why it waits. Without `after`,
        it answers at once; given `after`, the position of the value the reader
        last read, it waits for a change and answers the latest value, so values
        in between are coalesced. The value whose `last` is true ends the
        sequence: the Run ended, and its `lifecycle` and `stop_reason` say how,
        or no engine works it (`writer: none`) until a start with its Run ID
        resumes it. A Run another engine works is read through that engine, the
        owner the Run index names, and one no engine works from its committed
        state; a read never takes the Run's writer lock. A Run ID the Run index
        does not list fails with `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        view = self._crawl_view

        async def reading() -> AsyncSequence[CrawlRunProgress]:
            return view.progress()

        try:
            sequence = self._loop.call(reading)
        except LoopStopped:
            raise closed_error("run.progress") from None
        return Sequence(self._loop, sequence, lambda: closed_error("run.progress"))

    def report(self) -> CrawlRunReport:
        """Reads the crawl Run's report, as `AsyncCrawlRunView.report`.

        Reads a Run's report: at once for a Run that ended, and at its end for
        a live one, which the read never ends. A Run another engine works is
        read through that engine, and one no engine works from its committed
        state, never taking its writer lock. A Run that has no live writer and
        has not ended fails with `run_interrupted`, whose `fix` names a start
        with its Run ID, and a Run ID the Run index does not list with
        `run_not_found`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        try:
            return self._loop.call(self._crawl_view.report)
        except LoopStopped:
            raise closed_error("run.report") from None

    def records(
        self, dataset: Dataset[R], *, after: int | None = None
    ) -> Sequence[DatasetRecord[R]]:
        """Reads `dataset`'s committed Records, as `AsyncCrawlRunView.records`.

        Reads the committed Records of one Dataset of a crawl's Run in number
        order, from the first, or after the number `after`, which resumes a
        reader from the last Record it handled. Each Record goes in a
        `$/record` notification with its number, its value, its request's ID
        and URL and when it was committed, one per credit granted with
        `$/credit`. While the Run is live it follows the Run, sending each
        Record once it commits; once the Run has ended, or has no live writer,
        and every Record is sent, it answers how many it sent and the last
        one's number. With `through`, such as the number `dataset.page`
        answers as the last committed, it reads no Record numbered past it and
        answers once it sent every Record through it, a live Run's included.
        Records are read from the Run's committed state on a connection of
        their own, so a reader that grants no credit never slows the crawl.
        With `wall` set to `true` it reads only the Records whose page was
        read past a Wall, which their Provenance marks, and with `false` only
        the others (ADR 0052). A Dataset the crawl does not declare fails with
        `dataset_unknown`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        view = self._crawl_view

        async def reading() -> AsyncSequence[DatasetRecord[R]]:
            return view.records(dataset, after=after)

        try:
            sequence = self._loop.call(reading)
        except LoopStopped:
            raise closed_error("run.records") from None
        return Sequence(self._loop, sequence, lambda: closed_error("run.records"))

    def rejected_records(
        self, dataset: Dataset[Any], *, after: int | None = None
    ) -> Sequence[RejectedRecord]:
        """Reads `dataset`'s rejection table, as `AsyncCrawlRunView` does.

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

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        view = self._crawl_view

        async def reading() -> AsyncSequence[RejectedRecord]:
            return view.rejected_records(dataset, after=after)

        try:
            sequence = self._loop.call(reading)
        except LoopStopped:
            raise closed_error("run.rejected_records") from None
        return Sequence(
            self._loop, sequence, lambda: closed_error("run.rejected_records")
        )

    def export(
        self,
        dataset: Dataset[Any],
        *,
        format: ExportFormat | str,
        compression: ExportCompression | str | None = None,
        wall: bool | None = None,
        settings: CrawlSettings | None = None,
    ) -> Export:
        """Exports `dataset`'s committed Records, as `AsyncCrawlRunView.export`.

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
        """
        exporting = functools.partial(
            self._crawl_view.export,
            dataset,
            format=format,
            compression=compression,
            wall=wall,
            settings=settings,
        )
        try:
            exported = self._loop.call(exporting)
        except LoopStopped:
            raise closed_error("run.export") from None
        return Export(self._loop, exported)

    def outcomes(
        self,
        *,
        kinds: Iterable[OutcomeKind] | None = None,
        labels: Iterable[str | Label[Any]] | None = None,
        codes: Iterable[str] | None = None,
        after: int | Literal["now"] | None = None,
    ) -> Sequence[CrawlRequestOutcome]:
        """Reads the crawl's request outcomes, as `AsyncCrawlRunView.outcomes`.

        Reads a crawl's outcomes from its committed state, in commit order: from
        the first, after a position, or with `after: "now"` from the next commit.
        Its filters, `kinds`, `labels` and `codes` (an Error code any of the
        request's failed attempts failed with), run in the engine. While the Run
        is live and nothing new matches, it waits for the next commit; a page
        says `ended` once the Run has ended, or has no live writer, and nothing
        more matches. A reader never slows the Run. A request `run.retry_failed`
        re-admits ends again, and its later outcome supersedes the earlier one
        by `request.id`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        view = self._crawl_view
        kind_list = None if kinds is None else list(kinds)
        label_list = None if labels is None else list(labels)
        code_list = None if codes is None else list(codes)

        async def reading() -> AsyncSequence[CrawlRequestOutcome]:
            return view.outcomes(
                kinds=kind_list,
                labels=label_list,
                codes=code_list,
                after=after,
            )

        try:
            sequence = self._loop.call(reading)
        except LoopStopped:
            raise closed_error("run.outcomes") from None
        return Sequence(self._loop, sequence, lambda: closed_error("run.outcomes"))

    def failed_requests(self, *, blocked: bool = False) -> list[CrawlRequestOutcome]:
        """Lists failed (and optionally blocked) requests.

        Reads from a crawl's committed state the requests whose latest outcome
        is `failed`, and with `blocked` also `blocked`, each with that outcome:
        exactly those `run.retry_failed` re-admits with the same option. A skip
        is a decision and is never listed.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        try:
            return self._loop.call(
                functools.partial(self._crawl_view.failed_requests, blocked=blocked)
            )
        except LoopStopped:
            raise closed_error("run.failed_requests") from None
