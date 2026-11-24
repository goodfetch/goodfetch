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
import inspect
import weakref
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any, Generic, TypeVar, cast, final, overload

from goodfetch_collie_client.generated.models import KeyValueStoreAttempt
from goodfetch_collie_client.generated.models import Locator as LocatorId
from goodfetch_collie_client.json_values import non_json
from goodfetch_collie_client.markers import warn_entry
from goodfetch_collie_client.raw_call import carried_id
from goodfetch_collie_client.schema import (
    SchemaCodec,
    SchemaRejectedError,
    ValueNotJSONError,
    schema_codec,
)
from goodfetch_collie_client.settings import settings_on_wire
from pydantic import BaseModel

from goodfetch_foxhound.generated.errors import (
    DatasetDuplicateError,
    DatasetKeyInvalidError,
    LabelDuplicateError,
    LabelUnhandledError,
    LabelUnknownError,
    PayloadInvalidError,
    RecordInvalidError,
    ResponseNotDocumentError,
)
from goodfetch_foxhound.generated.models import (
    CrawlRequestEnqueueLinksParams,
    CrawlRequestEnqueueParams,
    CrawlRequestLinksParams,
    CrawlRequestPushParams,
    DatasetDeclaration,
    InvalidRecord,
    LabelDeclaration,
    RecordIssue,
    RequestSeed,
    SitemapSeed,
)
from goodfetch_foxhound.generated.operations import (
    DEPRECATED_OPERATIONS,
    EXPERIMENTAL_OPERATIONS,
)

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Iterable, Sequence

    from goodfetch_collie_client.generated.handles import (
        AsyncLocator,
        AsyncPage,
        Locator,
        Page,
    )
    from goodfetch_collie_client.generated.models import PageId
    from goodfetch_collie_client.json_values import JSONObject, JSONValue
    from goodfetch_collie_client.key_value_store import (
        AsyncKeyValueStore,
        KeyValueStore,
    )
    from goodfetch_collie_client.schema import JSONSchema

    from goodfetch_foxhound.generated.models import (
        Enqueued,
        EnqueuedLinks,
        MatchedLink,
    )
    from goodfetch_foxhound.generated.operations import (
        AsyncFoxhoundOperations,
        FoxhoundOperations,
    )
    from goodfetch_foxhound.generated.settings import LabelSettings

__all__ = [
    "AsyncAttemptRun",
    "AsyncCrawl",
    "AsyncCrawlContext",
    "AttemptRun",
    "Crawl",
    "CrawlContext",
    "CrawlRequest",
    "Dataset",
    "Label",
    "seed",
    "sitemap",
]

P = TypeVar("P")
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
    """A handler's attempt at one work item: its lease, and the params of its calls.

    Both contexts send what it builds, the async one awaiting each call and
    the sync one on its Connection's private loop.
    """

    run_id: str
    """The Run its crawl request belongs to."""
    crawl_request_id: str
    """The crawl request it handles."""
    lease_epoch: int
    """The epoch of its lease, which each of its calls carries."""
    crawl: AsyncCrawl | Crawl
    """The crawl whose handler it runs."""
    page_id: PageId | None
    """The page it was given, against whose document a relative `enqueue` resolves."""

    def lease(self) -> dict[str, object]:
        """The fields naming its lease, which each of its calls carries."""
        return {
            "run_id": self.run_id,
            "crawl_request_id": self.crawl_request_id,
            "lease_epoch": self.lease_epoch,
        }

    def key_value_store_attempt(self) -> KeyValueStoreAttempt:
        """What names it in each call of its Run's Key-value store."""
        return KeyValueStoreAttempt(
            crawl_request_id=self.crawl_request_id, lease_epoch=self.lease_epoch
        )

    def enqueue_params(
        self, url: str, label: Label[Any], payload: object, options: dict[str, object]
    ) -> CrawlRequestEnqueueParams:
        """`enqueue`'s params: `url` under `label`, with its Payload and options.

        Raises:
            LabelUnknownError: `label` is another crawl's.
            PayloadInvalidError: `payload` is not JSON, or its Label's model
                refuses it.
        """
        on_page = {} if self.page_id is None else {"page_id": self.page_id}
        fields = {
            **self.lease(),
            **on_page,
            "url": url,
            "label": declared_label(self.crawl, label),
            **_payload_field(label, payload),
            **options,
        }
        return CrawlRequestEnqueueParams.model_construct(None, **fields)

    def links_params(self, locator: AsyncLocator | Locator) -> CrawlRequestLinksParams:
        """`crawl_request.links`'s params: the elements `locator` matches."""
        fields = {**self.lease(), "locator": _locator_id(locator)}
        return CrawlRequestLinksParams.model_construct(None, **fields)

    def enqueue_links_params(
        self,
        locator: AsyncLocator | Locator,
        label: Label[Any],
        payload: object,
        options: dict[str, object],
        answers: Sequence[object] | None = None,
    ) -> CrawlRequestEnqueueLinksParams:
        """`enqueue_links`'s params: the links `locator` matches, under `label`.

        `answers`, in `payload`'s place, are a Payload function's answers,
        one for each element `crawl_request.links` read, `None` leaving its
        link out.

        Raises:
            LabelUnknownError: `label` is another crawl's.
            PayloadInvalidError: `payload`, or one of `answers`, is not
                JSON, or its Label's model refuses it.
        """
        name = declared_label(self.crawl, label)
        payloads = (
            _payload_field(label, payload)
            if answers is None
            else {"payloads": [_link_payload(label, answer) for answer in answers]}
        )
        fields = {
            **self.lease(),
            "locator": _locator_id(locator),
            "label": name,
            **payloads,
            **options,
        }
        return CrawlRequestEnqueueLinksParams.model_construct(None, **fields)

    def push(self, dataset: Dataset[R], records: R | list[R]) -> _Push:
        """`push`'s Records, one or a list of them, as the program checked them.

        A Record that is not JSON, or that its Dataset's model would not
        parse back, is refused here (ADR 0103); a hand-written JSON Schema
        is the engine's to check, so no Record is checked twice against one.
        """
        # A Record is a JSON object, never a list, so a list is the Records
        # of one push.
        pushed: list[object] = (
            cast("list[object]", records) if isinstance(records, list) else [records]
        )
        codec = _record_codecs.get(dataset)
        held: list[tuple[int, JSONValue]] = []
        refused: list[InvalidRecord] = []
        for position, record in enumerate(pushed):
            try:
                held.append((position, _record_on_wire(codec, record)))
            except ValueNotJSONError as not_json:
                message = f"{not_json.found} is not JSON"
                refused.append(_invalid(position, [(not_json.path, message)]))
            except SchemaRejectedError as rejected:
                issues = [(issue.path, issue.message) for issue in rejected.issues]
                refused.append(_invalid(position, issues))
        params = (
            CrawlRequestPushParams.model_construct(
                None,
                **self.lease(),
                dataset=dataset.name,
                records=[record for _, record in held],
            )
            if held
            else None
        )
        return _Push(dataset.name, params, [position for position, _ in held], refused)


def _record_on_wire(codec: SchemaCodec[Any] | None, record: object) -> JSONValue:
    """`record` as the wire carries it, through its Dataset's codec when it has one.

    Raises:
        ValueNotJSONError: Something in it is not JSON.
        SchemaRejectedError: Its Dataset's model would not parse it back.
    """
    if codec is not None:
        return codec.encode(record)
    refused = non_json(record)
    if refused is not None:
        raise ValueNotJSONError(refused)
    return cast("JSONValue", record)


def _invalid(position: int, issues: list[tuple[str, str]]) -> InvalidRecord:
    """The Record at `position` of a push, refused for `issues`, each by its path."""
    return InvalidRecord(
        position=position,
        issues=[RecordIssue(path=path, message=message) for path, message in issues],
    )


@final
@dataclass(frozen=True)
class _Push:
    """One push's Records as the program checked them: sent or refused."""

    dataset: str
    params: CrawlRequestPushParams | None
    """The call that sends the Records that hold; none when none does."""
    positions: list[int]
    """Each sent Record's position in the push, in the order sent."""
    refused: list[InvalidRecord]
    """Each Record the program refused, in position order."""

    def refusal(self, engine: RecordInvalidError | None) -> RecordInvalidError:
        """The `record_invalid` of the Records the program refused, and the engine's.

        `engine` is the engine's `record_invalid` of the Records sent, when
        it refused any: it names each by its position among those sent, so
        each is named again by its position in the push.
        """
        sent = (
            []
            if engine is None or engine.typed_details is None
            else engine.typed_details.records
        )
        records = sorted(
            [
                *self.refused,
                *(
                    InvalidRecord(
                        position=(
                            self.positions[record.position]
                            if record.position < len(self.positions)
                            else record.position
                        ),
                        issues=record.issues,
                    )
                    for record in sent
                ),
            ],
            key=lambda record: record.position,
        )
        named = "; ".join(
            f"the Record at {record.position}: "
            + ", ".join(
                f"{issue.path or 'its root'}: {issue.message}"
                for issue in record.issues
            )
            for record in records
        )
        return RecordInvalidError(
            f"the Dataset {self.dataset} did not stage {named}",
            fix="Push Records its Dataset's schema accepts, made of JSON values "
            "only, each holding every field of its Dataset's key.",
            details={
                "dataset": self.dataset,
                "records": [record.model_dump(mode="json") for record in records],
            },
        )


@final
class AsyncAttemptRun:
    """A crawl's Run bound to one attempt of an `AsyncCrawl` handler: `ctx.run`.

    What is written through it is staged with the attempt, so it commits
    with the attempt's Records and enqueues when the handler returns, and is
    discarded with them when the attempt ends any other way (ADRs 0093 and
    0103).

    The trap: the program's own Run value, the one `start_crawl` answered,
    captured by a handler from the code around it, is not bound. What a
    handler writes through it commits at once, outside the attempt's lease,
    and again on every attempt that runs the handler, discarded ones
    included. Write through it only state that must outlive a failed
    attempt. A program never constructs one.
    """

    def __init__(self, run_id: str, key_value_store: AsyncKeyValueStore) -> None:
        """Takes the Run's ID and its Key-value store bound to the attempt."""
        self._id = run_id
        self.key_value_store = key_value_store
        """The Run's Key-value store, its writes staged with the attempt.

        A read sees the attempt's own writes first; other attempts and the
        program's own Run value see them once they commit, and `keys` lists
        committed keys only. Once the attempt ends, each call fails.
        """

    @property
    def id(self) -> str:
        """The Run's ID, read without a round trip."""
        return self._id


@final
class AttemptRun:
    """A crawl's Run bound to one attempt of a `Crawl` handler: `ctx.run`.

    What is written through it is staged with the attempt, and the program's
    own Run value, captured by a handler, is not bound, as `AsyncAttemptRun`
    says. A program never constructs one.
    """

    def __init__(self, run_id: str, key_value_store: KeyValueStore) -> None:
        """Takes the Run's ID and its Key-value store bound to the attempt."""
        self._id = run_id
        self.key_value_store = key_value_store
        """The Run's Key-value store, its writes staged with the attempt.

        A read sees the attempt's own writes first; other attempts and the
        program's own Run value see them once they commit, and `keys` lists
        committed keys only. Once the attempt ends, each call fails.
        """

    @property
    def id(self) -> str:
        """The Run's ID, read without a round trip."""
        return self._id


@final
class AsyncCrawlContext(Generic[P]):
    """What an `AsyncCrawl` handler's attempt at one work item holds (ADR 0093).

    `P` is the type of its Label's Payload. Once the attempt ends, or its
    Run is closed, each call fails.
    """

    def __init__(
        self,
        request: CrawlRequest[P],
        attempt: HandlerAttempt,
        run: AsyncAttemptRun,
        operations: AsyncFoxhoundOperations,
        page: AsyncPage | None,
    ) -> None:
        """Takes the attempt's request, lease and Run, that Run's calls and its page."""
        self._request = request
        self._attempt = attempt
        self._run = run
        self._operations = operations
        self._page = page

    @property
    def request(self) -> CrawlRequest[P]:
        """The crawl request the attempt handles, its Payload typed by its Label."""
        return self._request

    @property
    def run(self) -> AsyncAttemptRun:
        """The crawl's Run bound to this attempt, not the program's own Run value.

        Its Key-value store's writes are staged with the attempt (ADR 0093).
        """
        return self._run

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
        if self._page is None:
            raise _no_page(self._request)
        return self._page

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
        label: Label[M],
        payload: M,
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
        label: Label[JSONObject],
        payload: JSONObject,
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
            PayloadInvalidError: `payload` is not JSON, or its Label's model
                refuses it, each issue named by its path; nothing is sent.
        """
        warn_entry(
            "crawl_request.enqueue", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS
        )
        options = _request_options(method, body, priority, forefront, key)
        params = self._attempt.enqueue_params(url, label, payload, options)
        return await self._operations.crawl_request_enqueue(params)

    @overload
    async def enqueue_links(
        self,
        locator: AsyncLocator,
        label: Label[None],
        *,
        include: Sequence[str] | None = None,
        exclude: Sequence[str] | None = None,
        limit: int | None = None,
        priority: int | None = None,
    ) -> EnqueuedLinks: ...
    @overload
    async def enqueue_links(
        self,
        locator: AsyncLocator,
        label: Label[M],
        payload: M | Callable[[MatchedLink], M | Awaitable[M | None] | None],
        *,
        include: Sequence[str] | None = None,
        exclude: Sequence[str] | None = None,
        limit: int | None = None,
        priority: int | None = None,
    ) -> EnqueuedLinks: ...
    @overload
    async def enqueue_links(
        self,
        locator: AsyncLocator,
        label: Label[JSONObject],
        payload: JSONObject
        | Callable[[MatchedLink], JSONObject | Awaitable[JSONObject | None] | None],
        *,
        include: Sequence[str] | None = None,
        exclude: Sequence[str] | None = None,
        limit: int | None = None,
        priority: int | None = None,
    ) -> EnqueuedLinks: ...
    async def enqueue_links(
        self,
        locator: AsyncLocator,
        label: Label[Any],
        payload: object = None,
        *,
        include: Sequence[str] | None = None,
        exclude: Sequence[str] | None = None,
        limit: int | None = None,
        priority: int | None = None,
    ) -> EnqueuedLinks:
        """Adds each link `locator` matches to the crawl, under `label`.

        Adds every link a locator matches on the page a handler's attempt was
        given, a handler context's `enqueueLinks`, each a `GET` of its `href`
        resolved against the document's base URL (its `<base href>`, resolved
        against the URL the document was reached at after its redirects, or
        that URL itself), its query percent-encoded in the document's
        encoding as a browser encodes it, and answers each matched element in
        document order: added, with its crawl request's ID and URL; not
        added, with its reason, as `crawl_request.enqueue` answers; `no_link`
        for an element without an `href`; or `skipped_by_payload` for one
        whose entry in `payloads` is `null`. Every link takes `payload`, or
        its own entry of `payloads`, the program's Payload function's answers
        for the elements `crawl_request.links` read; a Payload that does not
        fit its Label fails the call with `payload_invalid` and adds none.
        `limit` adds at most that many links: once they are added, each later
        element whose link the Frontier would judge is not added, with
        `link_limit`. `include` and `exclude` only narrow the crawl's scope; a
        pattern the engine cannot match fails with `pattern_unsupported`. A
        page no fetch tier serves fails with `closed`, and an attempt whose
        lease no longer holds with `attempt_ended`.

        The links go under `label`, each with the Payload that follows it when
        the Label has one (ADR 0093): one Payload for every link, or a Payload
        function, plain or `async`, which reads each matched element first
        (`crawl_request.links`) and answers its link's Payload, or `None` to
        leave it out. `include` lists URL patterns, globs as
        `crawl.scope.include`'s, of which a link must match one, `exclude`
        those it must match none of, and `priority` places them in the
        Frontier as `enqueue`'s does.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            LabelUnknownError: `label` is another crawl's; nothing is sent.
            PayloadInvalidError: `payload`, or a Payload its function answers,
                is not JSON, or its Label's model refuses it, each issue named
                by its path; no link is added.
        """
        warn_entry(
            "crawl_request.enqueue_links",
            EXPERIMENTAL_OPERATIONS,
            DEPRECATED_OPERATIONS,
        )
        options = _links_options(include, exclude, limit, priority)
        answers: list[object] | None = None
        if callable(payload):
            declared_label(self._attempt.crawl, label)
            read = await self._operations.crawl_request_links(
                self._attempt.links_params(locator)
            )
            answers = []
            for link in read.links:
                answer = payload(link)
                answers.append(await answer if inspect.isawaitable(answer) else answer)
        params = self._attempt.enqueue_links_params(
            locator, label, payload, options, answers
        )
        return await self._operations.crawl_request_enqueue_links(params)

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

        Each Record is checked in the program first: one that is not JSON,
        or that its Dataset's model would not parse back, is never sent,
        and the call fails with `record_invalid` once the others are
        staged, naming each refused Record by its position in the push, the
        engine's refusals included. Caught, the attempt keeps what it
        staged; escaped, it fails the request, which is not retried. An
        empty list sends nothing, so it never fails with `invalid_params`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("crawl_request.push", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        push = self._attempt.push(dataset, records)
        if push.params is not None:
            try:
                await self._operations.crawl_request_push(push.params)
            except RecordInvalidError as engine:
                if not push.refused:
                    raise
                raise push.refusal(engine) from engine
        if push.refused:
            raise push.refusal(None)


@final
class CrawlContext(Generic[P]):
    """What a `Crawl` handler's attempt at one work item holds (ADR 0093).

    `P` is the type of its Label's Payload. Its handler runs on a worker
    thread of the Connection's pool, and each call waits there for its
    answer. Once the attempt ends, or its Run is closed, each call fails.
    """

    def __init__(
        self,
        request: CrawlRequest[P],
        attempt: HandlerAttempt,
        run: AttemptRun,
        operations: FoxhoundOperations,
        page: Page | None,
    ) -> None:
        """Takes the attempt's request, lease and Run, that Run's calls and its page."""
        self._request = request
        self._attempt = attempt
        self._run = run
        self._operations = operations
        self._page = page

    @property
    def request(self) -> CrawlRequest[P]:
        """The crawl request the attempt handles, its Payload typed by its Label."""
        return self._request

    @property
    def run(self) -> AttemptRun:
        """The crawl's Run bound to this attempt, not the program's own Run value.

        Its Key-value store's writes are staged with the attempt (ADR 0093).
        """
        return self._run

    @property
    def page(self) -> Page:
        """Already navigated.

        The handler runs once the request's navigation has happened, so a
        route set on this page applies from its next navigation. To keep a
        resource kind from loading on every request, set
        `browser.block_resources`, which acts before each navigation (ADR
        0117). The work item's page: a Collie `Page`, on every fetch tier
        (ADR 0093).

        Raises:
            ResponseNotDocumentError: The response is not a document, such
                as a PDF its Label's `crawl.content_types` lists.
        """
        if self._page is None:
            raise _no_page(self._request)
        return self._page

    @overload
    def enqueue(
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
    def enqueue(
        self,
        url: str,
        label: Label[M],
        payload: M,
        *,
        method: str | None = None,
        body: bytes | None = None,
        priority: int | None = None,
        forefront: bool | None = None,
        key: str | None = None,
    ) -> Enqueued: ...
    @overload
    def enqueue(
        self,
        url: str,
        label: Label[JSONObject],
        payload: JSONObject,
        *,
        method: str | None = None,
        body: bytes | None = None,
        priority: int | None = None,
        forefront: bool | None = None,
        key: str | None = None,
    ) -> Enqueued: ...
    def enqueue(
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
        """Adds a request to the crawl, as `AsyncCrawlContext.enqueue` does.

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

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            LabelUnknownError: `label` is another crawl's.
            PayloadInvalidError: `payload` is not JSON, or its Label's model
                refuses it, each issue named by its path; nothing is sent.
        """
        warn_entry(
            "crawl_request.enqueue", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS
        )
        options = _request_options(method, body, priority, forefront, key)
        params = self._attempt.enqueue_params(url, label, payload, options)
        return self._operations.crawl_request_enqueue(params)

    @overload
    def enqueue_links(
        self,
        locator: Locator,
        label: Label[None],
        *,
        include: Sequence[str] | None = None,
        exclude: Sequence[str] | None = None,
        limit: int | None = None,
        priority: int | None = None,
    ) -> EnqueuedLinks: ...
    @overload
    def enqueue_links(
        self,
        locator: Locator,
        label: Label[M],
        payload: M | Callable[[MatchedLink], M | None],
        *,
        include: Sequence[str] | None = None,
        exclude: Sequence[str] | None = None,
        limit: int | None = None,
        priority: int | None = None,
    ) -> EnqueuedLinks: ...
    @overload
    def enqueue_links(
        self,
        locator: Locator,
        label: Label[JSONObject],
        payload: JSONObject | Callable[[MatchedLink], JSONObject | None],
        *,
        include: Sequence[str] | None = None,
        exclude: Sequence[str] | None = None,
        limit: int | None = None,
        priority: int | None = None,
    ) -> EnqueuedLinks: ...
    def enqueue_links(
        self,
        locator: Locator,
        label: Label[Any],
        payload: object = None,
        *,
        include: Sequence[str] | None = None,
        exclude: Sequence[str] | None = None,
        limit: int | None = None,
        priority: int | None = None,
    ) -> EnqueuedLinks:
        """Adds each link `locator` matches, as `AsyncCrawlContext.enqueue_links` does.

        Adds every link a locator matches on the page a handler's attempt was
        given, a handler context's `enqueueLinks`, each a `GET` of its `href`
        resolved against the document's base URL (its `<base href>`, resolved
        against the URL the document was reached at after its redirects, or
        that URL itself), its query percent-encoded in the document's
        encoding as a browser encodes it, and answers each matched element in
        document order: added, with its crawl request's ID and URL; not
        added, with its reason, as `crawl_request.enqueue` answers; `no_link`
        for an element without an `href`; or `skipped_by_payload` for one
        whose entry in `payloads` is `null`. Every link takes `payload`, or
        its own entry of `payloads`, the program's Payload function's answers
        for the elements `crawl_request.links` read; a Payload that does not
        fit its Label fails the call with `payload_invalid` and adds none.
        `limit` adds at most that many links: once they are added, each later
        element whose link the Frontier would judge is not added, with
        `link_limit`. `include` and `exclude` only narrow the crawl's scope; a
        pattern the engine cannot match fails with `pattern_unsupported`. A
        page no fetch tier serves fails with `closed`, and an attempt whose
        lease no longer holds with `attempt_ended`.

        A Payload function reads each matched element first and answers its
        link's Payload, or `None` to leave it out.

        Experimental: outside semver until it is promoted (ADR 0030).

        Raises:
            LabelUnknownError: `label` is another crawl's; nothing is sent.
            PayloadInvalidError: `payload`, or a Payload its function answers,
                is not JSON, or its Label's model refuses it, each issue named
                by its path; no link is added.
        """
        warn_entry(
            "crawl_request.enqueue_links",
            EXPERIMENTAL_OPERATIONS,
            DEPRECATED_OPERATIONS,
        )
        options = _links_options(include, exclude, limit, priority)
        answers: list[object] | None = None
        if callable(payload):
            declared_label(self._attempt.crawl, label)
            read = self._operations.crawl_request_links(
                self._attempt.links_params(locator)
            )
            answers = [payload(link) for link in read.links]
        params = self._attempt.enqueue_links_params(
            locator, label, payload, options, answers
        )
        return self._operations.crawl_request_enqueue_links(params)

    def push(self, dataset: Dataset[R], records: R | list[R]) -> None:
        """Pushes one Record, or a list of them, as `AsyncCrawlContext.push` does.

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

        Each Record is checked in the program first: one that is not JSON,
        or that its Dataset's model would not parse back, is never sent,
        and the call fails with `record_invalid` once the others are
        staged, naming each refused Record by its position in the push, the
        engine's refusals included. Caught, the attempt keeps what it
        staged; escaped, it fails the request, which is not retried. An
        empty list sends nothing, so it never fails with `invalid_params`.

        Experimental: outside semver until it is promoted (ADR 0030).
        """
        warn_entry("crawl_request.push", EXPERIMENTAL_OPERATIONS, DEPRECATED_OPERATIONS)
        push = self._attempt.push(dataset, records)
        if push.params is not None:
            try:
                self._operations.crawl_request_push(push.params)
            except RecordInvalidError as engine:
                if not push.refused:
                    raise
                raise push.refusal(engine) from engine
        if push.refused:
            raise push.refusal(None)


def _no_page(request: CrawlRequest[Any]) -> ResponseNotDocumentError:
    """The error of a handler's page read when its response is not a document."""
    message = (
        f"the crawl request {request.id}'s response is not a document, so its "
        "handler has no page"
    )
    return ResponseNotDocumentError(message)


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
        Record: one pushed again under the same key replaces it. Declaring
        does no I/O.

        Raises:
            DatasetDuplicateError: The crawl already declares a Dataset of
                `name`.
            DatasetKeyInvalidError: `key` names a field that is no top-level
                field `schema` requires, so a Record could lack it.
        """
        crawl = _crawls[self]
        if any(declared.name == name for declared in crawl.datasets):
            raise DatasetDuplicateError(
                f"the Crawl already declares the Dataset {name}",
                fix="Declare each Dataset once, and use the Dataset the first "
                "`crawl.dataset` returned.",
                details={"dataset": name},
            )
        codec = None if schema is None else schema_codec(schema)
        if codec is not None and key is not None:
            _check_key(name, codec.wire_schema, key)
        dataset = Dataset[Any](name)
        declaration: dict[str, object] = {"name": name}
        if codec is not None:
            _record_codecs[dataset] = codec
            declaration["schema"] = codec.wire_schema
        if key is not None:
            declaration["key"] = list(key)
        crawl.datasets.append(DatasetDeclaration.model_validate(declaration))
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
    label: Label[M],
    payload: M,
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
    label: Label[JSONObject],
    payload: JSONObject,
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

    Raises:
        PayloadInvalidError: `payload` is not JSON, or its Label's model
            refuses it, each issue named by its path. The engine checks a
            Payload against a hand-written JSON Schema, and its length
            against `crawl.payload.max_size`, when the crawl starts.
    """
    fields: dict[str, object] = {
        "url": url,
        "label": label.name,
        **_payload_field(label, payload),
        **_request_options(method, body, priority, forefront, key),
    }
    return RequestSeed.model_validate(fields)


def sitemap(url: str, rules: Iterable[tuple[str, Label[None]]]) -> SitemapSeed:
    """A sitemap seed of a crawl: the entries of `url`'s sitemaps (ADR 0093).

    `url` names a sitemap, or a site's robots.txt, whose `Sitemap:` lines
    are read, and nested sitemap indexes are followed. Each `(pattern,
    label)` of `rules`, in order, is a glob as `crawl.scope.include` takes
    one and the Label it gives the entries it matches: the first that
    matches an entry picks its Label, and an entry no rule matches is not
    added, with `no_rule`. A sitemap entry carries no Payload, so a rule's
    Label takes none. The engine reads the sitemap once the Run is running.
    """
    return SitemapSeed.model_validate(
        {
            "sitemap": url,
            "rules": [
                {"pattern": pattern, "label": label.name} for pattern, label in rules
            ],
        }
    )


def _payload_field(label: Label[Any], payload: object) -> dict[str, JSONValue]:
    """The wire's `payload` field of `payload`, sent under `label`; none without one.

    Raises:
        PayloadInvalidError: `payload` is not JSON, or its Label's model's own
            parse refuses it (ADR 0082), each issue named by its path; a
            hand-written JSON Schema is the engine's to check.
    """
    codec = _payload_codecs.get(label)
    if codec is None:
        return {}
    try:
        return {"payload": codec.encode(payload)}
    except ValueNotJSONError as refused:
        issues = [{"path": refused.path, "message": f"{refused.found} is not JSON"}]
        raise _payload_refused(label.name, issues) from None
    except SchemaRejectedError as refused:
        issues = [
            {"path": issue.path, "message": issue.message} for issue in refused.issues
        ]
        raise _payload_refused(label.name, issues) from None


def _link_payload(label: Label[Any], answer: object) -> JSONValue:
    """The wire's `payloads` entry of a Payload function's `answer`; `None` skips it.

    Raises:
        PayloadInvalidError: `answer` is not JSON, or its Label's model
            refuses it.
    """
    if answer is None:
        return None
    return _payload_field(label, answer).get("payload")


def _locator_id(locator: AsyncLocator | Locator) -> LocatorId:
    """The `Locator` a locator's calls carry."""
    locator_id = carried_id(locator)
    if not isinstance(locator_id, LocatorId):
        message = "invariant: a locator's calls carry a `Locator`"
        raise TypeError(message)
    return locator_id


def _check_key(dataset: str, schema: JSONObject, key: Sequence[str]) -> None:
    """Refuses a Dataset's `key` when it names a field `schema` does not require.

    Raises:
        DatasetKeyInvalidError: A field of `key` is no top-level field the
            JSON Schema `schema` requires, so a Record could lack it, as the
            engine refuses it.
    """
    required = schema.get("required")
    requires = required if isinstance(required, list) else []
    missing = next((name for name in key if name not in requires), None)
    if missing is None:
        return
    raise DatasetKeyInvalidError(
        f"the Dataset {dataset}'s key names {missing}, which is no top-level field "
        "its schema requires",
        fix="Key a Dataset only by fields its schema requires, or require the "
        "field in its schema.",
        details={"dataset": dataset, "reason": "not_required", "field": missing},
    )


def _payload_refused(label: str, issues: list[dict[str, str]]) -> PayloadInvalidError:
    """The `payload_invalid` of a Payload for the Label `label` refused for `issues`."""
    named = "; ".join(
        f"{issue['path'] or 'its root'}: {issue['message']}" for issue in issues
    )
    return PayloadInvalidError(
        f"the Payload for the Label {label} was not sent: {named}",
        fix="Send a Payload its Label's schema accepts, made of JSON values only: "
        "a string, a finite number, a boolean, None, a list or a dict with "
        "string keys.",
        details={"label": label, "issues": issues},
    )


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


def _links_options(
    include: Sequence[str] | None,
    exclude: Sequence[str] | None,
    limit: int | None,
    priority: int | None,
) -> dict[str, object]:
    """The wire's fields of `enqueue_links`'s options, each one given."""
    options: dict[str, object] = {}
    if include is not None:
        options["include"] = list(include)
    if exclude is not None:
        options["exclude"] = list(exclude)
    if limit is not None:
        options["limit"] = limit
    if priority is not None:
        options["priority"] = priority
    return options


def check_startable(
    crawl: AsyncCrawl | Crawl, seeds: Sequence[RequestSeed | SitemapSeed]
) -> None:
    """Fails, before anything is sent, when `crawl` cannot start from `seeds`.

    Raises:
        LabelUnhandledError: A Label has no handler.
        LabelUnknownError: A seed, or a sitemap seed's rule, names a Label
            `crawl` does not declare, by name, since a seed carries its
            Label's name only.
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
        named = (
            [rule.label for rule in seed.rules]
            if isinstance(seed, SitemapSeed)
            else [seed.label]
        )
        for name in named:
            if name not in declared.labels:
                raise declared.unknown(name)


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
        SchemaValidatorFailedError: A validator of the Label's schema raised
            something other than a rejection.
    """
    declared = _crawls[crawl].labels.get(label)
    codec = None if declared is None else _payload_codecs.get(declared.label)
    return None if codec is None else codec.decode(payload)
