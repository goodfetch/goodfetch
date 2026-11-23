"""A crawl as its program declares it (ADR 0093, as ADRs 0082, 0099 and 0103 amend it).

An `AsyncCrawl`, whose handlers are `async def` functions, or a sync
`Crawl`, whose handlers are plain functions, holds its Labels, the handler
of each and its Datasets, and no connection: it is declared at module
scope, declaring does no I/O, and two Runs started from it share nothing.
`label` types each Label by its Payload, `@crawl.handle(label)` attaches a
handler and returns it unchanged, so a test calls it as written, and
`dataset` types each Dataset by its Records. The two are siblings, neither
a subtype of the other, so a type checker refuses an `AsyncCrawl` where a
`Crawl` goes. The engine leases each work item to the handler of its
Label, which reads its crawl request, with its typed Payload, from its
context. TypeScript's `crawl.ts` is its twin.
"""

from __future__ import annotations

import base64
import weakref
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any, Generic, TypeVar, cast, final, overload

from goodfetch_collie_client.schema import SchemaCodec, schema_codec
from goodfetch_collie_client.settings import settings_on_wire
from pydantic import BaseModel

from goodfetch_foxhound.generated.errors import (
    LabelDuplicateError,
    LabelUnhandledError,
    LabelUnknownError,
    ResponseNotDocumentError,
)
from goodfetch_foxhound.generated.models import (
    CrawlRequestEnqueueParams,
    CrawlRequestPushParams,
    DatasetDeclaration,
    LabelDeclaration,
    RequestSeed,
)

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Sequence

    from goodfetch_collie_client.generated.handles import AsyncPage
    from goodfetch_collie_client.json_values import JSONObject, JSONValue
    from goodfetch_collie_client.schema import JSONSchema

    from goodfetch_foxhound.generated.models import Enqueued
    from goodfetch_foxhound.generated.operations import AsyncFoxhoundOperations
    from goodfetch_foxhound.generated.settings import LabelSettings

__all__ = [
    "AsyncCrawl",
    "AsyncCrawlContext",
    "Crawl",
    "CrawlContext",
    "CrawlRequest",
    "Dataset",
    "Label",
    "seed",
]

P = TypeVar("P")
Q = TypeVar("Q")
R = TypeVar("R")
M = TypeVar("M", bound=BaseModel)


@final
class Label(Generic[P]):
    """A kind of page a crawl handles (ADR 0093).

    Each request names one, and the handler attached to it handles its
    pages. `P` is the type of the Payload its requests carry: a Pydantic
    model, a JSON object for a hand-written JSON Schema, or `None` for a
    Label without one. `crawl.label` declares one; a program never
    constructs one.
    """

    def __init__(self, name: str) -> None:
        """Takes its name."""
        self._name = name

    @property
    def name(self) -> str:
        """The name the Label was declared with, unique in its crawl."""
        return self._name


@final
class Dataset(Generic[R]):
    """A named collection of Records a crawl's handlers push (ADR 0103).

    `R` is the type of each Record: a Pydantic model, or a JSON object
    without a schema or for a hand-written one. `crawl.dataset` declares
    one; a program never constructs one.
    """

    def __init__(self, name: str) -> None:
        """Takes its name."""
        self._name = name

    @property
    def name(self) -> str:
        """The name the Dataset was declared with, unique in its crawl."""
        return self._name


@final
@dataclass(frozen=True)
class CrawlRequest(Generic[P]):
    """The crawl request a handler's attempt handles: one work item (ADR 0093).

    `payload` is typed by its Label, `None` for a Label without one.
    """

    id: str
    """Its crawl request's ID."""
    url: str
    """The URL it was admitted at."""
    label: str
    """The name of its Label."""
    method: str
    """Its HTTP method, `GET` unless its enqueue named another."""
    payload: P
    """The Payload its program attached, typed by its Label."""


@dataclass(frozen=True)
class HandlerAttempt:
    """What a handler's attempt calls the engine with: its lease and its page."""

    run_id: str
    """The Run its crawl request belongs to."""
    crawl_request_id: str
    """The crawl request it handles."""
    lease_epoch: int
    """The epoch of its lease, which each of its calls carries."""
    crawl: AsyncCrawl | Crawl
    """The crawl whose handler it runs."""
    operations: AsyncFoxhoundOperations
    """Foxhound's calls, sent on its Run."""
    page: AsyncPage | None
    """Its page; none for a response that is not a document."""

    def lease(self) -> dict[str, object]:
        """The fields naming its lease, which each of its calls carries."""
        return {
            "run_id": self.run_id,
            "crawl_request_id": self.crawl_request_id,
            "lease_epoch": self.lease_epoch,
        }


@final
class AsyncCrawlContext(Generic[P]):
    """What an `AsyncCrawl` handler's attempt at one work item holds (ADR 0093).

    `P` is the type of its Label's Payload. Once the attempt ends, or its
    Run is closed, each call fails.
    """

    def __init__(self, request: CrawlRequest[P], attempt: HandlerAttempt) -> None:
        """Takes the crawl request of the attempt, and what it calls with."""
        self._request = request
        self._attempt = attempt

    @property
    def request(self) -> CrawlRequest[P]:
        """The crawl request the attempt handles, its Payload typed by its Label."""
        return self._request

    @property
    def page(self) -> AsyncPage:
        """Already navigated.

        The handler runs once the request's navigation has happened, so a
        route set on this page applies from its next navigation. To keep a
        resource kind from loading on every request, set
        `browser.block_resources`, which acts before each navigation (ADR
        0117). The work item's page: a Collie `AsyncPage`, on every fetch
        tier (ADR 0093).

        Raises:
            ResponseNotDocumentError: The response is not a document, such
                as a PDF its Label's `crawl.content_types` lists.
        """
        page = self._attempt.page
        if page is None:
            message = (
                f"the crawl request {self._request.id}'s response is not a "
                "document, so its handler has no page"
            )
            raise ResponseNotDocumentError(message)
        return page

    @overload
    async def enqueue(
        self,
        url: str,
        label: Label[None],
        *,
        method: str | None = None,
        body: bytes | None = None,
        priority: int | None = None,
        forefront: bool | None = None,
        key: str | None = None,
    ) -> Enqueued: ...
    @overload
    async def enqueue(
        self,
        url: str,
        label: Label[Q],
        payload: Q,
        *,
        method: str | None = None,
        body: bytes | None = None,
        priority: int | None = None,
        forefront: bool | None = None,
        key: str | None = None,
    ) -> Enqueued: ...
    async def enqueue(
        self,
        url: str,
        label: Label[Any],
        payload: object = None,
        *,
        method: str | None = None,
        body: bytes | None = None,
        priority: int | None = None,
        forefront: bool | None = None,
        key: str | None = None,
    ) -> Enqueued:
        """Adds a request to the crawl, handled under `label`, with its Payload.

        Adds one request to a crawl from inside a handler's attempt, a
        handler context's `enqueue`, and answers what the Frontier did with
        it: added, with its crawl request's ID and its URL as parsed, or not
        added, with its reason (`duplicate`, `out_of_scope`,
        `unsupported_scheme`, `invalid_url`, `max_depth`, `max_requests`,
        `path_budget` or a URL cap's name). A relative URL resolves against
        the document's base URL: its `<base href>`, resolved against the URL
        the document was reached at after its redirects, or that URL itself.
        A request is `duplicate` when one of its identity was already
        admitted: its method, its URL's key (the fragment, `utm_*`, `gclid`,
        `fbclid`, empty pairs and `;jsessionid=` dropped, the parameters
        sorted) and its body's hash, or the `key` the program declares in
        their place; it is still fetched at its own URL. The request sits one
        level deeper than the one whose attempt added it. An attempt whose
        lease no longer holds fails with `attempt_ended`.

        The request goes under `label`, and its Payload comes after the
        Label only when the Label has one (ADR 0082).

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            LabelUnknownError: `label` is another crawl's.
        """
        attempt = self._attempt
        name = declared_label(attempt.crawl, label)
        fields = {
            **attempt.lease(),
            "url": url,
            "label": name,
            **_payload_field(label, payload),
            **_request_options(method, body, priority, forefront, key),
        }
        return await attempt.operations.crawl_request_enqueue(
            CrawlRequestEnqueueParams.model_construct(None, **fields)
        )

    async def push(self, dataset: Dataset[R], records: R | list[R]) -> None:
        """Pushes one Record, or a list of them, to `dataset` (ADR 0103).

        Pushes one Record or several, in one message, into a Dataset the
        crawl declares from inside a handler's attempt, a handler context's
        `push`, and answers once they are staged: they commit once with the
        attempt's other outputs when the handler returns, and are discarded
        with them when the attempt ends any other way. Each Record is checked
        first. A push of no Record fails with `invalid_params`. A Dataset the
        crawl does not declare fails with `dataset_unknown`, and none of the
        Records is staged. A Record that is no JSON object, fails its
        Dataset's JSON Schema or lacks one of its key's fields is not staged:
        its rejection is staged instead, entering the Dataset's rejection
        table (`run.rejected_records`) if the attempt commits, while the
        Records that hold are staged and the attempt keeps what it staged
        before; the call then fails with `record_invalid`, naming each such
        Record by its position in the list and each of its issues by its
        path. An attempt that already ended fails with `attempt_ended`.

        An empty list sends nothing, so it never fails with
        `invalid_params`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        # A Record is a JSON object, never a list, so a list is the Records
        # of one push.
        pushed: list[object] = (
            cast("list[object]", records) if isinstance(records, list) else [records]
        )
        if not pushed:
            return
        codec = _record_codecs.get(dataset)
        wire = [record if codec is None else codec.encode(record) for record in pushed]
        params = CrawlRequestPushParams.model_construct(
            None, **self._attempt.lease(), dataset=dataset.name, records=wire
        )
        await self._attempt.operations.crawl_request_push(params)


@final
class CrawlContext(Generic[P]):
    """What a `Crawl` handler's attempt at one work item holds (ADR 0093).

    `P` is the type of its Label's Payload. Once the attempt ends, or its
    Run is closed, each call fails.
    """

    def __init__(self, request: CrawlRequest[P]) -> None:
        """Takes the crawl request of the attempt."""
        self._request = request

    @property
    def request(self) -> CrawlRequest[P]:
        """The crawl request the attempt handles, its Payload typed by its Label."""
        return self._request


@dataclass
class _Declared:
    """One Label a crawl declares: its handle, its settings and its handler."""

    label: Label[Any]
    settings: LabelSettings | None
    handler: Callable[..., object] | None = None


@dataclass
class _Crawl:
    """What one crawl declares: its Labels by name and its Datasets."""

    labels: dict[str, _Declared] = field(default_factory=dict[str, _Declared])
    datasets: list[DatasetDeclaration] = field(default_factory=list[DatasetDeclaration])

    def declared(self, label: Label[Any]) -> _Declared:
        """The declaration of `label`, the very Label this crawl's `label` returned.

        Raises:
            LabelUnknownError: `label` is another crawl's.
        """
        declared = self.labels.get(label.name)
        if declared is None or declared.label is not label:
            raise self.unknown(label.name)
        return declared

    def unknown(self, name: str) -> LabelUnknownError:
        """The `label_unknown` of a Label `name` this crawl does not declare."""
        return LabelUnknownError(
            f"the Crawl does not declare the Label {name}",
            fix="Use a Label this crawl's `crawl.label` returned, or declare it "
            "on this crawl.",
            details={"label": name, "declared": list(self.labels)},
        )


_crawls: weakref.WeakKeyDictionary[_Declaration, _Crawl] = weakref.WeakKeyDictionary()
"""What each crawl declares, kept off its public members."""

_payload_codecs: weakref.WeakKeyDictionary[Label[Any], SchemaCodec[Any]] = (
    weakref.WeakKeyDictionary()
)
"""Each Label's Payload codec; a Label without a Payload has none."""

_record_codecs: weakref.WeakKeyDictionary[Dataset[Any], SchemaCodec[Any]] = (
    weakref.WeakKeyDictionary()
)
"""Each Dataset's Records codec; a Dataset without a schema has none."""


class _Declaration:
    """What `AsyncCrawl` and `Crawl` declare alike: Labels and Datasets."""

    def __init__(self) -> None:
        _crawls[self] = _Crawl()

    @overload
    def label(
        self, name: str, *, settings: LabelSettings | None = None
    ) -> Label[None]: ...
    @overload
    def label(
        self, name: str, *, payload: type[M], settings: LabelSettings | None = None
    ) -> Label[M]: ...
    @overload
    def label(
        self, name: str, *, payload: JSONSchema, settings: LabelSettings | None = None
    ) -> Label[JSONObject]: ...
    def label(
        self,
        name: str,
        *,
        payload: type[BaseModel] | JSONSchema | None = None,
        settings: LabelSettings | None = None,
    ) -> Label[Any]:
        """Declares a Label, whose requests carry Payloads of `payload` (ADR 0082).

        `payload` is a Pydantic model class or a JSON Schema written by hand,
        which the engine receives as JSON Schema; a Label without one carries
        none. `settings` are the Label's own over its Run's, such as
        `{"fetch": {"tier": "http"}}` (ADR 0093).

        Raises:
            LabelDuplicateError: The crawl already declares a Label of `name`.
        """
        crawl = _crawls[self]
        if name in crawl.labels:
            message = f"the Crawl already declares the Label {name}"
            raise LabelDuplicateError(
                message,
                fix="Declare each Label once, and use the Label the first "
                "`crawl.label` returned.",
                details={"label": name},
            )
        label = Label[Any](name)
        if payload is not None:
            _payload_codecs[label] = schema_codec(payload)
        crawl.labels[name] = _Declared(label, settings)
        return label

    @overload
    def dataset(
        self, name: str, *, key: Sequence[str] | None = None
    ) -> Dataset[JSONObject]: ...
    @overload
    def dataset(
        self, name: str, *, schema: type[M], key: Sequence[str] | None = None
    ) -> Dataset[M]: ...
    @overload
    def dataset(
        self, name: str, *, schema: JSONSchema, key: Sequence[str] | None = None
    ) -> Dataset[JSONObject]: ...
    def dataset(
        self,
        name: str,
        *,
        schema: type[BaseModel] | JSONSchema | None = None,
        key: Sequence[str] | None = None,
    ) -> Dataset[Any]:
        """Declares a Dataset, whose Records are of `schema` (ADR 0103).

        `schema` is a Pydantic model class or a JSON Schema written by hand,
        which the engine checks each Record against; without one, a Record
        is any JSON object. `key` names the fields whose values name a
        Record: one pushed again under the same key replaces it. A second
        Dataset of one name fails the Run's start with `dataset_duplicate`.
        """
        dataset = Dataset[Any](name)
        declaration: dict[str, object] = {"name": name}
        if schema is not None:
            codec = schema_codec(schema)
            _record_codecs[dataset] = codec
            declaration["schema"] = codec.wire_schema
        if key is not None:
            declaration["key"] = list(key)
        _crawls[self].datasets.append(DatasetDeclaration.model_validate(declaration))
        return dataset

    def _attach(self, label: Label[Any], handler: Callable[..., object]) -> None:
        """Attaches `handler` to `label`, which this crawl must declare.

        Raises:
            LabelUnknownError: `label` is another crawl's.
            LabelDuplicateError: `label` already has a handler.
        """
        declared = _crawls[self].declared(label)
        if declared.handler is not None:
            message = f"the Label {label.name} already has a handler"
            raise LabelDuplicateError(
                message,
                fix="Attach one handler to each Label.",
                details={"label": label.name},
            )
        declared.handler = handler


@final
class AsyncCrawl(_Declaration):
    """A crawl's declaration whose handlers are `async def` functions (ADR 0093).

    `AsyncFoxhound` starts it, and each handler runs as a task of the Run's
    task group. It holds no connection and does no I/O.
    """

    def handle(
        self, label: Label[P]
    ) -> Callable[
        [Callable[[AsyncCrawlContext[P]], Awaitable[None]]],
        Callable[[AsyncCrawlContext[P]], Awaitable[None]],
    ]:
        """Attaches the decorated `async def` function to `label`, returned unchanged.

        The engine leases each of the Label's work items to it.

        Raises:
            LabelUnknownError: `label` is another crawl's.
            LabelDuplicateError: `label` already has a handler.
        """

        def attach(
            handler: Callable[[AsyncCrawlContext[P]], Awaitable[None]],
        ) -> Callable[[AsyncCrawlContext[P]], Awaitable[None]]:
            self._attach(label, handler)
            return handler

        return attach


@final
class Crawl(_Declaration):
    """A crawl's declaration whose handlers are plain functions (ADR 0093).

    `Foxhound` starts it, and each handler runs on the Connection's thread
    pool. It holds no connection and does no I/O.
    """

    def handle(
        self, label: Label[P]
    ) -> Callable[
        [Callable[[CrawlContext[P]], None]], Callable[[CrawlContext[P]], None]
    ]:
        """Attaches the decorated function to `label` and returns it unchanged.

        The engine leases each of the Label's work items to it.

        Raises:
            LabelUnknownError: `label` is another crawl's.
            LabelDuplicateError: `label` already has a handler.
        """

        def attach(
            handler: Callable[[CrawlContext[P]], None],
        ) -> Callable[[CrawlContext[P]], None]:
            self._attach(label, handler)
            return handler

        return attach


def crawl_declaration(
    crawl: AsyncCrawl | Crawl,
) -> tuple[dict[str, LabelDeclaration], list[DatasetDeclaration]]:
    """The Labels and Datasets `crawl.start` declares for `crawl`."""
    declared = _crawls[crawl]
    labels: dict[str, LabelDeclaration] = {}
    for name, label in declared.labels.items():
        # Only what the Label declares is sent: a field set to `None` would
        # go as `null`.
        declaration: dict[str, object] = {}
        codec = _payload_codecs.get(label.label)
        if codec is not None:
            declaration["payload"] = codec.wire_schema
        if label.settings is not None:
            declaration["settings"] = settings_on_wire(label.settings)
        labels[name] = LabelDeclaration.model_validate(declaration)
    return labels, list(declared.datasets)


@overload
def seed(
    url: str,
    label: Label[None],
    *,
    method: str | None = None,
    body: bytes | None = None,
    priority: int | None = None,
    forefront: bool | None = None,
    key: str | None = None,
) -> RequestSeed: ...
@overload
def seed(
    url: str,
    label: Label[Q],
    payload: Q,
    *,
    method: str | None = None,
    body: bytes | None = None,
    priority: int | None = None,
    forefront: bool | None = None,
    key: str | None = None,
) -> RequestSeed: ...
def seed(
    url: str,
    label: Label[Any],
    payload: object = None,
    *,
    method: str | None = None,
    body: bytes | None = None,
    priority: int | None = None,
    forefront: bool | None = None,
    key: str | None = None,
) -> RequestSeed:
    """A seed request of a crawl: `url`, handled under `label` (ADR 0093).

    A Label with a Payload takes one, after it (ADR 0082). `method` is
    `GET` when left out, `body` is sent with it, a higher `priority` is
    fetched first, `forefront` puts it ahead of the requests of its
    priority, and `key` names it in place of its method, URL and body, so
    another of the same key is a duplicate.
    """
    fields: dict[str, object] = {
        "url": url,
        "label": label.name,
        **_payload_field(label, payload),
        **_request_options(method, body, priority, forefront, key),
    }
    return RequestSeed.model_validate(fields)


def _payload_field(label: Label[Any], payload: object) -> dict[str, JSONValue]:
    """The wire's `payload` field of `payload`, sent under `label`; none without one."""
    codec = _payload_codecs.get(label)
    return {} if codec is None else {"payload": codec.encode(payload)}


def _request_options(
    method: str | None,
    body: bytes | None,
    priority: int | None,
    forefront: bool | None,
    key: str | None,
) -> dict[str, object]:
    """The wire's fields of a request's options, each one given."""
    options: dict[str, object] = {}
    if method is not None:
        options["method"] = method
    if body is not None:
        options["body"] = base64.b64encode(body).decode("ascii")
    if priority is not None:
        options["priority"] = priority
    if forefront is not None:
        options["forefront"] = forefront
    if key is not None:
        options["key"] = key
    return options


def check_startable(crawl: AsyncCrawl | Crawl, seeds: Sequence[RequestSeed]) -> None:
    """Fails, before anything is sent, when `crawl` cannot start from `seeds`.

    Raises:
        LabelUnhandledError: A Label has no handler.
        LabelUnknownError: A seed names a Label `crawl` does not declare, by
            name, since a seed carries its Label's name only.
    """
    declared = _crawls[crawl]
    for name, label in declared.labels.items():
        if label.handler is None:
            message = f"the Crawl attaches no handler to the Label {name}"
            raise LabelUnhandledError(
                message,
                fix="Attach one with `@crawl.handle(label)`, or stop declaring "
                "the Label.",
                details={"label": name},
            )
    for seed in seeds:
        if seed.label not in declared.labels:
            raise declared.unknown(seed.label)


def declared_label(crawl: AsyncCrawl | Crawl, label: Label[Any]) -> str:
    """The name of `label`, which `crawl` must declare.

    Raises:
        LabelUnknownError: `label` is another crawl's.
    """
    return _crawls[crawl].declared(label).label.name


def handler_of(crawl: AsyncCrawl | Crawl, label: str) -> Callable[..., object] | None:
    """The handler attached to the Label named `label` in `crawl`, if any."""
    declared = _crawls[crawl].labels.get(label)
    return None if declared is None else declared.handler


def payload_of(crawl: AsyncCrawl | Crawl, label: str, payload: JSONValue) -> object:
    """The Payload a crawl request under `label` carries, as its Label types it.

    A Label without a Payload carries `None`.

    Raises:
        SchemaRejectedError: The Label's schema rejects it.
    """
    declared = _crawls[crawl].labels.get(label)
    codec = None if declared is None else _payload_codecs.get(declared.label)
    return None if codec is None else codec.decode(payload)
