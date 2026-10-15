"""Hooks and SDK handlers in code (ADR 0104, decided in #481).

A crawl declares its Hooks in code on its `Crawl` or `AsyncCrawl`, one decorator per stage, which returns the function
unchanged so a test calls it as it is. Hooks run in the order declared, after the Hooks that settings name (`hooks.*`),
with no integers. Scoped to `labels`, a Hook sees their Payload typed; left unscoped, it runs for every Label and sees
`object`. The observe-only stages (the run's lifecycle and a request's end) reach code through the Run's own values (its
progress, outcomes and report, ADR 0102). A
Collie program has no Hooks in code: its own lines, routes, init scripts and locator handlers answer each stage
(ADR 0085).

A Hook returns its change as a value and ends the work with a call that never returns, as a handler's `skip` and `retry`
do (ADR 0093). A program's own implementation of a Plugin kind, an SDK handler, is registered on `connect()` in one
argument per kind keyed by name, as ADR 0101 registers `model_providers`, and settings choose it by that name as they
choose an installed Plugin.
"""

from collections.abc import Awaitable, Callable, Mapping, Sequence
from dataclasses import dataclass
from typing import Any, Generic, Literal, NoReturn, TypeAlias, TypeVar, final, overload

from goodfetch._crawl import CrawlRequest, FetchTier
from goodfetch._data import Dataset, JsonObject, Label
from goodfetch._lifecycle import AsyncCrawlRun, AsyncPage, CrawlRun, Page
from goodfetch._page import AsyncResponse, Response
from goodfetch._results import BlockCause, ErrorCode, GoodfetchError, ProgramError
from goodfetch._sessions import SecretPlaceholder, SecretReference

P = TypeVar("P")
T = TypeVar("T")

HookStage: TypeAlias = Literal[
    "before_request", "after_response", "before_navigation", "after_navigation", "page_ready", "before_record_commit"
]
"""The stages a Hook in code runs at, by their catalog names."""

ReportedHookStage: TypeAlias = HookStage | Literal["request_ended"] | str
"""A stage as reported: also the observe-only ones only Plugins and Script Hooks reach, and a newer engine's (G3)."""

class _HookContext(Generic[P]):
    """What every Hook's context carries. A context belongs to one call: once the Hook has returned or ended the work,
    calls on it raise `AttemptEndedError`."""

    @property
    def request(self) -> CrawlRequest[P]: ...

class _BeforeRequestBase(_HookContext[P]):
    @property
    def tier(self) -> FetchTier | str:
        """The tier about to fetch: every attempt, escalation and sample-comparison run comes here again."""
    @property
    def method(self) -> str:
        """The method the request was enqueued with, `GET` by default (ADR 0093). A Hook reads it and never changes it."""
    @property
    def body(self) -> bytes | None:
        """The body the request was enqueued with, or `None`. A Hook reads it (to sign the request, say), never changes it."""
    @property
    def headers(self) -> Mapping[str, str]:
        """The headers as built so far, the enqueue's own and the Hooks before this one included. Cookies, Secret values and the Session's
        credentials are added after the last Hook, so none of them is here."""
    @property
    def previous_error(self) -> GoodfetchError | None:
        """The error of the attempt before this one; `None` on the first."""
    def skip(self, message: str) -> NoReturn:
        """Ends the request `skipped`, with the reason `hook`; its outcome still lists every failed attempt's error."""

@final
class BeforeRequestContext(_BeforeRequestBase[P]):
    """Before each fetch of a request, for a sync crawl: also the stage before a retry."""

    @property
    def run(self) -> CrawlRun:
        """For its Key-value store (#479), which Script Hooks have too (ADR 0051)."""

@final
class AsyncBeforeRequestContext(_BeforeRequestBase[P]):
    @property
    def run(self) -> AsyncCrawlRun: ...

@final
@dataclass(frozen=True, kw_only=True)
class RequestChange:
    """What a before-request Hook changes: the URL and headers, never the method or the body the request was enqueued
    with (ADR 0093)."""

    url: str | None = None
    """A new URL, checked against the crawl's scope and the run's domain policy (not an enqueue's other checks); a Hook in
    code may reach any host that admits. robots.txt and Site preferences
    read it as rewritten."""
    headers: Mapping[str, str | SecretPlaceholder | None] = ...
    """Headers to set, or `None` to remove one. `Host`, `Cookie`, `Authorization`, `Proxy-*` and method overrides fail
    the Hook with `header_forbidden`. A value may be a Secret placeholder, checked against the request's URL (ADR 0092)."""

class _AfterResponseBase(_HookContext[P]):
    @property
    def tier(self) -> FetchTier | str: ...
    def block(self, cause: BlockCause, message: str | None = None) -> NoReturn:
        """Ends the attempt `blocked` with this cause: ADR 0021 escalates by new Session, and the request ends `blocked` once
        escalation runs out."""
    def retry(self, message: str) -> NoReturn:
        """Ends this attempt with `retry_requested`, retried under `crawl.retry.max`, as a handler's `retry`."""

@final
class AfterResponseContext(_AfterResponseBase[P]):
    """After the final response, before ADR 0093's status rule: where a soft block served with a 200 is marked."""

    @property
    def response(self) -> Response: ...
    @property
    def run(self) -> CrawlRun: ...

@final
class AsyncAfterResponseContext(_AfterResponseBase[P]):
    @property
    def response(self) -> AsyncResponse: ...
    @property
    def run(self) -> AsyncCrawlRun: ...

@final
@dataclass(frozen=True, kw_only=True)
class ResponseChange:
    body: str | bytes
    """A rewritten body, which the handler's page is served from."""

@final
class PageStageContext(_HookContext[P]):
    """Before and after a navigation (only on the `browser` and `light_browser` tiers), and when the page is ready, just
    before the handler, on every tier. Page calls follow the handler's tier rules (ADR 0093)."""

    @property
    def tier(self) -> FetchTier | str: ...
    @property
    def page(self) -> Page: ...
    @property
    def run(self) -> CrawlRun: ...

@final
class AsyncPageStageContext(_HookContext[P]):
    @property
    def tier(self) -> FetchTier | str: ...
    @property
    def page(self) -> AsyncPage: ...
    @property
    def run(self) -> AsyncCrawlRun: ...

class _RecordCommitBase(_HookContext[P], Generic[P, T]):
    @property
    def dataset(self) -> Dataset[T]: ...
    @property
    def record(self) -> T: ...
    def drop(self, reason: str) -> NoReturn:
        """Leaves the Record out, counted in the run's report with this reason."""
    def reject(self, reason: str) -> NoReturn:
        """Puts the Record in the Dataset's rejection table, read with `run.rejected_records` (ADR 0103)."""

@final
class RecordCommitContext(_RecordCommitBase[P, T]):
    """Before a Record's commit, with the attempt's other outputs (ADR 0003). The engine checks the Record it commits
    against the Dataset's schema."""

    @property
    def run(self) -> CrawlRun: ...

@final
class AsyncRecordCommitContext(_RecordCommitBase[P, T]):
    @property
    def run(self) -> AsyncCrawlRun: ...

BeforeRequestHook: TypeAlias = Callable[[BeforeRequestContext[P]], RequestChange | None]
AsyncBeforeRequestHook: TypeAlias = Callable[[AsyncBeforeRequestContext[P]], Awaitable[RequestChange | None]]
AfterResponseHook: TypeAlias = Callable[[AfterResponseContext[P]], ResponseChange | None]
AsyncAfterResponseHook: TypeAlias = Callable[[AsyncAfterResponseContext[P]], Awaitable[ResponseChange | None]]
PageStageHook: TypeAlias = Callable[[PageStageContext[P]], None]
AsyncPageStageHook: TypeAlias = Callable[[AsyncPageStageContext[P]], Awaitable[None]]
RecordCommitHook: TypeAlias = Callable[[RecordCommitContext[P, T]], T | None]
"""Returns the Record to commit, changed or not; `None` commits it unchanged."""
AsyncRecordCommitHook: TypeAlias = Callable[[AsyncRecordCommitContext[P, T]], Awaitable[T | None]]

class SyncHooks:
    """The Hook decorators of a sync `Crawl`. With `labels` of one Payload type, a Hook sees it typed; unscoped, or
    scoped to Labels of several types, it sees `object`. A Label of another crawl raises `LabelUnknownError` at once, a
    Dataset of another crawl `DatasetUnknownError`, and an empty `labels` or `datasets` list `HookIneffectiveError`. `name` identifies
    the Hook in `hook_failed`, the Run trace and a test's `run_hook`, unique on its crawl or `HookDuplicateError` at once:
    by default the function's `__qualname__`, or the stage and position (`before_request#2`) for a `<lambda>`."""

    @overload
    def before_request(self, *, labels: Sequence[Label[P]], name: str | None = None) -> Callable[[BeforeRequestHook[P]], BeforeRequestHook[P]]: ...
    @overload
    def before_request(
        self, *, labels: Sequence[Label[Any]] | None = None, name: str | None = None
    ) -> Callable[[BeforeRequestHook[object]], BeforeRequestHook[object]]: ...
    @overload
    def after_response(self, *, labels: Sequence[Label[P]], name: str | None = None) -> Callable[[AfterResponseHook[P]], AfterResponseHook[P]]: ...
    @overload
    def after_response(
        self, *, labels: Sequence[Label[Any]] | None = None, name: str | None = None
    ) -> Callable[[AfterResponseHook[object]], AfterResponseHook[object]]: ...
    @overload
    def before_navigation(self, *, labels: Sequence[Label[P]], name: str | None = None) -> Callable[[PageStageHook[P]], PageStageHook[P]]: ...
    @overload
    def before_navigation(
        self, *, labels: Sequence[Label[Any]] | None = None, name: str | None = None
    ) -> Callable[[PageStageHook[object]], PageStageHook[object]]: ...
    @overload
    def after_navigation(self, *, labels: Sequence[Label[P]], name: str | None = None) -> Callable[[PageStageHook[P]], PageStageHook[P]]: ...
    @overload
    def after_navigation(
        self, *, labels: Sequence[Label[Any]] | None = None, name: str | None = None
    ) -> Callable[[PageStageHook[object]], PageStageHook[object]]: ...
    @overload
    def page_ready(self, *, labels: Sequence[Label[P]], name: str | None = None) -> Callable[[PageStageHook[P]], PageStageHook[P]]: ...
    @overload
    def page_ready(
        self, *, labels: Sequence[Label[Any]] | None = None, name: str | None = None
    ) -> Callable[[PageStageHook[object]], PageStageHook[object]]: ...
    @overload
    def before_record_commit(
        self, *, datasets: Sequence[Dataset[T]], labels: Sequence[Label[Any]] | None = None, name: str | None = None
    ) -> Callable[[RecordCommitHook[object, T]], RecordCommitHook[object, T]]:
        """Scoped to `datasets`, a Hook sees and returns their Record type."""
    @overload
    def before_record_commit(
        self, *, labels: Sequence[Label[Any]] | None = None, name: str | None = None
    ) -> Callable[[RecordCommitHook[object, JsonObject]], RecordCommitHook[object, JsonObject]]: ...

class AsyncHooks:
    """The Hook decorators of an `AsyncCrawl`, for `async def` Hooks: `SyncHooks` with async contexts."""

    @overload
    def before_request(
        self, *, labels: Sequence[Label[P]], name: str | None = None
    ) -> Callable[[AsyncBeforeRequestHook[P]], AsyncBeforeRequestHook[P]]: ...
    @overload
    def before_request(
        self, *, labels: Sequence[Label[Any]] | None = None, name: str | None = None
    ) -> Callable[[AsyncBeforeRequestHook[object]], AsyncBeforeRequestHook[object]]: ...
    @overload
    def after_response(
        self, *, labels: Sequence[Label[P]], name: str | None = None
    ) -> Callable[[AsyncAfterResponseHook[P]], AsyncAfterResponseHook[P]]: ...
    @overload
    def after_response(
        self, *, labels: Sequence[Label[Any]] | None = None, name: str | None = None
    ) -> Callable[[AsyncAfterResponseHook[object]], AsyncAfterResponseHook[object]]: ...
    @overload
    def before_navigation(
        self, *, labels: Sequence[Label[P]], name: str | None = None
    ) -> Callable[[AsyncPageStageHook[P]], AsyncPageStageHook[P]]: ...
    @overload
    def before_navigation(
        self, *, labels: Sequence[Label[Any]] | None = None, name: str | None = None
    ) -> Callable[[AsyncPageStageHook[object]], AsyncPageStageHook[object]]: ...
    @overload
    def after_navigation(
        self, *, labels: Sequence[Label[P]], name: str | None = None
    ) -> Callable[[AsyncPageStageHook[P]], AsyncPageStageHook[P]]: ...
    @overload
    def after_navigation(
        self, *, labels: Sequence[Label[Any]] | None = None, name: str | None = None
    ) -> Callable[[AsyncPageStageHook[object]], AsyncPageStageHook[object]]: ...
    @overload
    def page_ready(self, *, labels: Sequence[Label[P]], name: str | None = None) -> Callable[[AsyncPageStageHook[P]], AsyncPageStageHook[P]]: ...
    @overload
    def page_ready(
        self, *, labels: Sequence[Label[Any]] | None = None, name: str | None = None
    ) -> Callable[[AsyncPageStageHook[object]], AsyncPageStageHook[object]]: ...
    @overload
    def before_record_commit(
        self, *, datasets: Sequence[Dataset[T]], labels: Sequence[Label[Any]] | None = None, name: str | None = None
    ) -> Callable[[AsyncRecordCommitHook[object, T]], AsyncRecordCommitHook[object, T]]: ...
    @overload
    def before_record_commit(
        self, *, labels: Sequence[Label[Any]] | None = None, name: str | None = None
    ) -> Callable[[AsyncRecordCommitHook[object, JsonObject]], AsyncRecordCommitHook[object, JsonObject]]: ...

# The Error codes of Hooks (ADR 0104).

@final
@dataclass(frozen=True, kw_only=True)
class HookFailedDetails:
    """The Hook, then the exception as `handler_failed` carries a handler's."""

    stage: ReportedHookStage
    hook: str
    """The Hook's name."""
    label: str | None
    """`None` at a lifecycle stage, which has no request."""
    exception_type: str
    exception_message: str
    stack: Sequence[str]
    wrapped_code: ErrorCode | None
    """The Goodfetch code raised inside the Hook, whose retryable flag this error carries; `None` for the program's own
    exception."""

@final
class HookFailedError(ProgramError):
    """A Hook raised. For the program's own exception, not retried unless `crawl.retry.handler_failed` is on; for a
    Goodfetch code raised inside, retried when that code is. At the Record stage the attempt's commit fails and
    everything it staged is discarded. `__cause__` holds the exception in the process that raised it."""

    @property
    def details(self) -> HookFailedDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class HookIneffectiveDetails:
    hook: str
    stage: ReportedHookStage

@final
class HookIneffectiveError(ProgramError):
    """A Hook that can never run: an empty `labels` or `datasets` list, at once, or, when the Run starts, a navigation
    Hook all of whose Labels force a tier that does not navigate (`http`, `discovered_api`, `browser_fetch`)."""

    @property
    def details(self) -> HookIneffectiveDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class HookDuplicateDetails:
    hook: str

@final
class HookDuplicateError(ProgramError):
    """A second Hook of one name on one crawl, raised at once."""

    @property
    def details(self) -> HookDuplicateDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class HeaderForbiddenDetails:
    header: str

@final
class HeaderForbiddenError(ProgramError):
    """A before-request Hook set a header the engine owns (`Host`, `Cookie`, `Authorization`, `Proxy-*`, a method
    override). Raised inside `hook_failed`."""

    @property
    def details(self) -> HeaderForbiddenDetails: ...

# SDK handlers: a program's own implementation of a Plugin kind (ADR 0033), registered on `connect()` in one argument
# per kind, each a mapping keyed by name (`model_providers` is ADR 0101's, `resolve_secret` ADR 0092's; Hooks are
# declared on the `Crawl`). Settings choose one by name as they choose an installed Plugin. The only one of a kind that
# one setting chooses (Fetch backend, Proxy provider) becomes that setting's value at the connection scope, with the
# run-layer origin `registered`, the lowest: every other run-layer value replaces it, configuration does not, and a lock
# wins with no refusal. Two of such a kind fail `connect()` with `SettingInvalidError` when nothing else sets the key,
# as does a handler named as a built-in or an installed Plugin. Collie's SDK has no `fetch_backends` or `sinks` (ADR
# 0088). Each kind's handler is generated from its plugin-protocol method group (area 7's spec): the shapes below are
# provisional. A `GoodfetchError` it raises keeps its code; any other exception is its kind's `<kind>_handler_failed`,
# whose message names it, as ADR 0101's `model_handler_failed`. The async client takes coroutine functions, run in the
# connection's task group; the sync client takes plain functions, run on its thread pool (ADR 0081).

@final
@dataclass(frozen=True, kw_only=True)
class BackendRequest:
    """A target the engine already checked under the page class and the run's domain policy (ADR 0033)."""

    url: str
    method: str
    """The crawl request's own method (ADR 0093)."""
    headers: Mapping[str, str]
    body: bytes | None
    """The crawl request's own body, or `None`."""
    proxy: str | None
    """The Session's proxy server, with no credentials, or `None`: a proxied crawl does not leave from the user's own
    address."""

@final
@dataclass(frozen=True, kw_only=True)
class BackendResponse:
    status: int
    headers: Mapping[str, str]
    body: bytes

AsyncFetchBackendHandler: TypeAlias = Callable[[BackendRequest], Awaitable[BackendResponse]]
"""The HTTP tiers' transport. It never carries a request that uses a Secret or a logged-in identity, such as a
logged-in Session's, one whose Hook set a Secret placeholder, one whose Session has `session.http_credentials` or one
whose proxy has a password: the engine sends those itself, so no logged-in cookie or Secret value passes through the
program. Other requests carry an anonymous Session's cookies in `headers`, and a response's `Set-Cookie` updates the
Session."""
FetchBackendHandler: TypeAlias = Callable[[BackendRequest], BackendResponse]

@final
@dataclass(frozen=True, kw_only=True)
class ProxyRequest:
    """One proxy per Session (ADRs 0021, 0092)."""

    session_id: str
    domain: str
    """The registrable domain of the Session's first request."""
    replacing: BlockCause | None
    """Set when the Session replaces one retired for this Block cause."""

@final
@dataclass(frozen=True, kw_only=True)
class ProxyAnswer:
    """A proxy, with Playwright's fields as `proxy.*` has them (ADR 0092)."""

    server: str
    username: str | None = None
    password: SecretReference | None = None
    """A Secret reference, never a value: `caller:` resolves through `resolve_secret`."""

AsyncProxyProviderHandler: TypeAlias = Callable[[ProxyRequest], Awaitable[ProxyAnswer]]
ProxyProviderHandler: TypeAlias = Callable[[ProxyRequest], ProxyAnswer]

@final
@dataclass(frozen=True, kw_only=True)
class SinkDelivery:
    """One delivery to a Sink handler (ADR 0008): the same delivery ID on each retry, so a write keyed by it lands once."""

    delivery_id: str
    run_epoch: int
    dataset: str
    records: Sequence[JsonObject]
    """Committed Records, in commit order."""

AsyncSinkHandler: TypeAlias = Callable[[SinkDelivery], Awaitable[None]]
"""A Sink written in the program (ADR 0026's SDK-handler form), serving the `sinks.<name>` entry of its name, which gives
its Datasets, write mode and key; a `type` there beside a handler of that name is refused when the Run starts. A failed
delivery is retried on ADR 0008's schedule; its Guarantee class is probed as any Sink's (ADR 0026)."""
SinkHandler: TypeAlias = Callable[[SinkDelivery], None]

@final
@dataclass(frozen=True, kw_only=True)
class SdkHandlerFailedDetails:
    """The handler, then the exception as `handler_failed` carries a handler's."""

    handler: str
    """The name it was registered under."""
    exception_type: str
    exception_message: str
    stack: Sequence[str]

@final
class FetchHandlerFailedError(ProgramError):
    """`fetch_handler_failed`: in a crawl, retried only when `crawl.retry.handler_failed` is on for the request's Label."""

    @property
    def details(self) -> SdkHandlerFailedDetails: ...

@final
class ProxyHandlerFailedError(ProgramError):
    """`proxy_handler_failed`: the Session does not open. In a crawl, the requests that needed it are retried only when
    `crawl.retry.handler_failed` is on for their Label; in a Collie Run, the call fails."""

    @property
    def details(self) -> SdkHandlerFailedDetails: ...

@final
class SinkHandlerFailedError(ProgramError):
    """`sink_handler_failed`: the delivery is retried on ADR 0008's schedule."""

    @property
    def details(self) -> SdkHandlerFailedDetails: ...
