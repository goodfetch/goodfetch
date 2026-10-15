"""Foxhound's connection (ADRs 0090, 0095): `AsyncFoxhound` and `Foxhound` carry every member of `AsyncCollie` and
`Collie` under the same names, Collie's Run start included, plus their own, so moving a program from Collie to Foxhound
changes its import and its connection type only. Each finds Foxhound's executable in its own wheel and reads only
`GOODFETCH_FOXHOUND_ENGINE_PATH` (ADR 0090). A crawl's Run, `AsyncCrawlRun`, is a Run with a crawl's members.
"""

from collections.abc import Generator, Mapping, Sequence
from os import PathLike
from types import TracebackType
from typing import Any, Generic, Literal, TypeVar, final, overload

from goodfetch_collie_client import (
    AsyncConnection,
    AsyncEventIterator,
    AsyncModelProviderHandler,
    AsyncProxyProviderHandler,
    AsyncResolveSecret,
    AsyncRun,
    AsyncRunView,
    Connection,
    EngineChoice,
    EventIterator,
    ExplainedSetting,
    Limits,
    ModelProviderHandler,
    NotGiven,
    ProxyProviderHandler,
    Recording,
    ResolveSecret,
    Run,
    RunView,
    Timeout,
)
from goodfetch_foxhound._crawl import AsyncCrawl, Crawl, Enqueued, Seed, SitemapSeed
from goodfetch_foxhound._data import Dataset, Label
from goodfetch_foxhound._hooks import AsyncFetchBackendHandler, AsyncSinkHandler, FetchBackendHandler, SinkHandler
from goodfetch_foxhound._outcomes import CrawlRequestOutcome
from goodfetch_foxhound._output import AsyncExport, DatasetRecord, Export, ExportFormat, RejectedRecord
from goodfetch_foxhound._run import CrawlRunProgress, CrawlRunReport
from goodfetch_foxhound._settings import CrawlRunSettings, SettingKey, Settings

_T = TypeVar("_T")
_P = TypeVar("_P")

_C = TypeVar("_C")

@final
class _AsyncConnecting(Generic[_C]):
    """What `connect()` returns: awaited, or used with `async with`."""

    def __await__(self) -> Generator[None, None, _C]: ...
    async def __aenter__(self) -> _C: ...
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

@final
class AsyncCrawlRun(AsyncRun):
    """A crawl's Run (ADRs 0093, 0102); its Records are ADR 0103's. Its handlers run from its start until it ends. It is
    a Run (ADR 0099), so it also opens pages and Sessions of the program's own, and its report and progress add a crawl's."""

    async def finish(self, *, timeout: Timeout = ...) -> CrawlRunReport: ...
    async def stop(self, *, timeout: Timeout = ...) -> CrawlRunReport: ...
    def progress(self) -> AsyncEventIterator[CrawlRunProgress]: ...
    async def explain_settings(self, keys: Sequence[SettingKey] = ..., *, timeout: Timeout = ...) -> list[ExplainedSetting]:
        """All the scope's settings when `keys` is left out, Foxhound's keys included."""

    @property
    def seeds(self) -> Sequence[Enqueued]:
        """Each seed's answer, given once at the start: added with its ID, or not added with its reason. On a resume, the
        answers given when the Run began."""
    def outcomes(
        self,
        *,
        kinds: Sequence[Literal["succeeded", "failed", "skipped", "blocked"]] | None = None,
        labels: Sequence[Label[Any]] | None = None,
        after: Literal["now"] | None = None,
    ) -> AsyncEventIterator[CrawlRequestOutcome]:
        """Every ending in commit order, a request ending again after `retry_failed`: a log of endings, not current
        state; `failed_requests()` reads the current failures.

        Read from the Run's committed state from its first, then following new commits until the Run ends; the filters
        run in the engine (ADR 0081). A reader of committed state, it never slows the Run. A request's new outcome
        supersedes the old: key outcomes by `request.id`."""
    def failed_requests(self, *, blocked: bool = False) -> AsyncEventIterator[CrawlRequestOutcome]:
        """The crawl requests whose latest outcome is `failed` (with `blocked=True`, also those whose latest is
        `blocked`), each as that outcome, read from committed state at the call and ending after what is committed:
        exactly what `retry_failed` with the same option would re-admit (ADR 0117). Never slows the Run."""
    async def retry_failed(self, *, blocked: bool = False, timeout: Timeout = ...) -> int:
        """Re-admits, in this same Run, every request that ended `failed`, and with `blocked=True` those that ended
        `blocked`, each keeping its ID, Label, Payload and depth, its attempts counting on with a fresh `crawl.retry.max`.
        A skip is a decision and is never re-admitted. On an ended Run it returns the Run to `running`, its Verdict and
        Completeness given anew when it ends (ADR 0029). Returns how many it re-admitted."""
    def records(self, dataset: Dataset[_T], *, after: int = 0) -> AsyncEventIterator[DatasetRecord[_T]]:
        """The Dataset's committed Records, from its first (or after `after`), following the Run live until it ends and
        every Record is read (ADR 0103). A cursor over run state: reading slowly, or not at all, never slows the crawl.
        One the native parse rejects ends the loop with `RecordInvalidError` carrying its number. Through a handler's
        `ctx.run`, it reads what has committed and ends, since the Run cannot end while the handler holds its request."""
    def rejected_records(self, dataset: Dataset[Any], *, after: int = 0) -> AsyncEventIterator[RejectedRecord]:
        """The Dataset's rejection table, each entry numbered in commit order, following the Run as `records` does."""
    async def export(
        self, dataset: Dataset[Any], *, format: ExportFormat, compression: Literal["gzip", "zstd"] | None = None, timeout: Timeout = ...
    ) -> AsyncExport:
        """Writes an Export of the Records committed so far (ADR 0026), to save or stream wherever the engine runs.

        Returns only once its file is whole; a write the engine cannot make raises `FileWriteFailedError` with
        `side="engine"` and leaves no unfinished file (ADR 0116).
        """
    @overload
    async def add_seed(self, seed: SitemapSeed, /, *, timeout: Timeout = ...) -> Enqueued:
        """Adds a built seed, such as `sitemap(url, rules)`; a sitemap's entries matching no rule are reported as `no_rule` in
        the Run's report (ADR 0102)."""
    @overload
    async def add_seed(
        self, url: str, label: Label[None], *, method: str = "GET", headers: Mapping[str, str] | None = None, body: str | bytes | None = None, priority: int = 0, forefront: bool = False, key: str | None = None, timeout: Timeout = ...
    ) -> Enqueued: ...
    @overload
    async def add_seed(
        self,
        url: str,
        label: Label[_P],
        payload: _P,
        *,
        method: str = "GET",
        headers: Mapping[str, str] | None = None,
        body: str | bytes | None = None,
        priority: int = 0,
        forefront: bool = False,
        key: str | None = None,
        timeout: Timeout = ...,
    ) -> Enqueued:
        """Adds a seed to the running crawl (ADR 0037's CO-03), committed at once, or staged with the attempt when called
        through a handler's `ctx.run`. A seed widens the scope by its host, where a handler's `enqueue` is checked against
        it; one the domain policy refuses is not added (`domain_policy`)."""

@final
class AsyncCrawlRunView(AsyncRunView[CrawlRunReport, CrawlRunProgress]):
    """A crawl's Run read by a program that does not drive it, opened with `open_crawl_run` (ADR 0102). Its Records are
    read with the program's own Dataset handles."""

    def outcomes(
        self,
        *,
        kinds: Sequence[Literal["succeeded", "failed", "skipped", "blocked"]] | None = None,
        labels: Sequence[Label[Any]] | None = None,
        after: Literal["now"] | None = None,
    ) -> AsyncEventIterator[CrawlRequestOutcome]: ...
    def failed_requests(self, *, blocked: bool = False) -> AsyncEventIterator[CrawlRequestOutcome]:
        """As `AsyncCrawlRun.failed_requests`, on a Run this program does not drive."""
    def records(self, dataset: Dataset[_T], *, after: int = 0) -> AsyncEventIterator[DatasetRecord[_T]]: ...
    def rejected_records(self, dataset: Dataset[Any], *, after: int = 0) -> AsyncEventIterator[RejectedRecord]: ...
    async def export(
        self, dataset: Dataset[Any], *, format: ExportFormat, compression: Literal["gzip", "zstd"] | None = None, timeout: Timeout = ...
    ) -> AsyncExport: ...

@final
class AsyncFoxhound(AsyncConnection):
    """A connection to a Foxhound engine: the entry point (ADR 0078). A Collie engine refuses it at `connect()` with
    `not_in_product` (ADR 0095). It carries every member of `AsyncCollie` under the same names, plus Foxhound's own.
    """

    @classmethod
    def connect(
        cls,
        *,
        engine: EngineChoice = "auto",
        settings: Settings = ...,
        env: Mapping[str, str] = ...,
        project: str | PathLike[str] | None | NotGiven = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
        resolve_secret: AsyncResolveSecret | None = None,
        model_providers: Mapping[str, AsyncModelProviderHandler] | None = None,
        fetch_backends: Mapping[str, AsyncFetchBackendHandler] | None = None,
        proxy_providers: Mapping[str, AsyncProxyProviderHandler] | None = None,
        sinks: Mapping[str, AsyncSinkHandler] | None = None,
    ) -> _AsyncConnecting[AsyncFoxhound]:
        """Starts or finds the engine and completes `hello` before the connection is returned.

        Left at `auto`, `engine` is filled by `GOODFETCH_URL` when set; a value written here wins over it. `settings` holds
        every other setting for this connection and everything beneath it. `env` is the environment map, read once here
        and never at import, by default the process's own (ADRs 0062, 0069): only the deployment variables are read, an
        unknown `GOODFETCH_` name fails with `SettingUnknownError`, and an Attached engine gets exactly this map.
        `project` is the project configuration file: left out, found upward from the working directory as the CLI does;
        `None`, none (ADR 0083). `resolve_secret` answers `caller:` Secret references from the program's own store, at
        the moment of use (ADR 0092). `model_providers` are Model providers answered by the program's own code, by the
        name Model references use: `"mine/any-model"` for `{"mine": handler}`, from the Runs this connection opens; a
        name taken by a built-in, a declared provider or a Plugin raises `SettingInvalidError` (ADR 0101).
        `fetch_backends`, `proxy_providers` and `sinks` are the program's own Plugins of those kinds, SDK handlers by
        the name settings choose them by (ADR 0104): the only Fetch backend or Proxy provider becomes its choosing
        setting's value with the run-layer origin `registered`, which every other run-layer value replaces, two fail
        unless `settings` choose one, and a Sink handler serves the `sinks.<name>` entry of its name.
        """
    async def explain_settings(self, keys: Sequence[SettingKey] = ..., *, timeout: Timeout = ...) -> list[ExplainedSetting]:
        """All the scope's settings when `keys` is left out, Foxhound's keys included."""
    async def start_crawl(
        self,
        crawl: AsyncCrawl,
        *,
        seeds: Sequence[Seed[Any] | SitemapSeed],
        run_id: str | None = None,
        settings: CrawlRunSettings = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
        plan: str | PathLike[str] | None = None,
        recording: Recording | None = None,
    ) -> AsyncCrawlRun:
        """Leaving the scope or `close()` stops the Run with a Checkpoint; `finish()` ends it.

        Starts a Run of the crawl from its seeds, which define its scope, and returns once it is started; its handlers
        run from then on (ADR 0102, Firecrawl's `start_crawl`). Raises `LabelUnhandledError` when a Label has no
        handler, and `LabelUnknownError` when a resumed Run has pending requests of a Label no longer declared (ADR
        0093). `settings` are the Run's, the place for a crawl-wide value such as `browser.block_resources`, since a
        `Crawl` takes none.

        `run_id` names the Run for good: a start with an existing Run ID resumes the Run if it was interrupted, or
        returns it as it ended (ADR 0102); a new ID starts a new Run. An existing Run behaves as `start`'s does, the
        seeds and the Plan file being inputs that must match.

        Fails with `model_price_unknown` (`ModelPriceUnknownError`) when a currency Limit or `ai.allowance.price` applies
        and the Run's `ai.model` has no price, unless `plan.frozen` (ADR 0117).
        """
    async def open_crawl_run(self, run_id: str, *, timeout: Timeout = ...) -> AsyncCrawlRunView:
        """Opens a crawl's Run by its ID to read it, as `open_run` does, with its outcomes, Records and
        Exports."""

# Sync API: the same concepts, generated from the same catalog (ADR 0038).

@final
class CrawlRun(Run):
    """A crawl's Run (ADRs 0093, 0102); its Records are ADR 0103's. Its handlers run on the connection's thread pool from
    its start until it ends. It is a Run (ADR 0099)."""

    def finish(self, *, timeout: Timeout = ...) -> CrawlRunReport: ...
    def stop(self, *, timeout: Timeout = ...) -> CrawlRunReport: ...
    def progress(self) -> EventIterator[CrawlRunProgress]: ...
    def explain_settings(self, keys: Sequence[SettingKey] = ..., *, timeout: Timeout = ...) -> list[ExplainedSetting]: ...

    @property
    def seeds(self) -> Sequence[Enqueued]:
        """Each seed's answer, given once at the start: added with its ID, or not added with its reason. On a resume, the
        answers given when the Run began."""
    def outcomes(
        self,
        *,
        kinds: Sequence[Literal["succeeded", "failed", "skipped", "blocked"]] | None = None,
        labels: Sequence[Label[Any]] | None = None,
        after: Literal["now"] | None = None,
    ) -> EventIterator[CrawlRequestOutcome]: ...
    def failed_requests(self, *, blocked: bool = False) -> EventIterator[CrawlRequestOutcome]: ...
    def retry_failed(self, *, blocked: bool = False, timeout: Timeout = ...) -> int: ...
    def records(self, dataset: Dataset[_T], *, after: int = 0) -> EventIterator[DatasetRecord[_T]]: ...
    def rejected_records(self, dataset: Dataset[Any], *, after: int = 0) -> EventIterator[RejectedRecord]: ...
    def export(
        self, dataset: Dataset[Any], *, format: ExportFormat, compression: Literal["gzip", "zstd"] | None = None, timeout: Timeout = ...
    ) -> Export: ...
    @overload
    def add_seed(self, seed: SitemapSeed, /, *, timeout: Timeout = ...) -> Enqueued: ...
    @overload
    def add_seed(
        self, url: str, label: Label[None], *, method: str = "GET", headers: Mapping[str, str] | None = None, body: str | bytes | None = None, priority: int = 0, forefront: bool = False, key: str | None = None, timeout: Timeout = ...
    ) -> Enqueued: ...
    @overload
    def add_seed(
        self,
        url: str,
        label: Label[_P],
        payload: _P,
        *,
        method: str = "GET",
        headers: Mapping[str, str] | None = None,
        body: str | bytes | None = None,
        priority: int = 0,
        forefront: bool = False,
        key: str | None = None,
        timeout: Timeout = ...,
    ) -> Enqueued: ...

@final
class CrawlRunView(RunView[CrawlRunReport, CrawlRunProgress]):
    """`AsyncCrawlRunView` in the sync API."""

    def outcomes(
        self,
        *,
        kinds: Sequence[Literal["succeeded", "failed", "skipped", "blocked"]] | None = None,
        labels: Sequence[Label[Any]] | None = None,
        after: Literal["now"] | None = None,
    ) -> EventIterator[CrawlRequestOutcome]: ...
    def failed_requests(self, *, blocked: bool = False) -> EventIterator[CrawlRequestOutcome]: ...
    def records(self, dataset: Dataset[_T], *, after: int = 0) -> EventIterator[DatasetRecord[_T]]: ...
    def rejected_records(self, dataset: Dataset[Any], *, after: int = 0) -> EventIterator[RejectedRecord]: ...
    def export(
        self, dataset: Dataset[Any], *, format: ExportFormat, compression: Literal["gzip", "zstd"] | None = None, timeout: Timeout = ...
    ) -> Export: ...

@final
class Foxhound(Connection):
    """`AsyncFoxhound` in the sync API.

    It runs the async core on a private background thread of its own, so it works on any thread, including one whose event
    loop is running (a notebook cell); its values are safe to call from any thread (ADR 0081).
    """

    @classmethod
    def connect(
        cls,
        *,
        engine: EngineChoice = "auto",
        settings: Settings = ...,
        env: Mapping[str, str] = ...,
        project: str | PathLike[str] | None | NotGiven = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
        resolve_secret: ResolveSecret | None = None,
        model_providers: Mapping[str, ModelProviderHandler] | None = None,
        fetch_backends: Mapping[str, FetchBackendHandler] | None = None,
        proxy_providers: Mapping[str, ProxyProviderHandler] | None = None,
        sinks: Mapping[str, SinkHandler] | None = None,
    ) -> Foxhound:
        """Starts or finds the engine and completes `hello` before returning; use it with `with`.

        Left at `auto`, `engine` is filled by `GOODFETCH_URL` when set; a value written here wins over it. `settings` holds
        every other setting for this connection and everything beneath it. `env` is the environment map, read once here
        and never at import, by default the process's own (ADRs 0062, 0069): only the deployment variables are read, an
        unknown `GOODFETCH_` name fails with `SettingUnknownError`, and an Attached engine gets exactly this map.
        `project` is the project configuration file: left out, found upward from the working directory as the CLI does;
        `None`, none (ADR 0083). `model_providers` are Model providers answered by the program's own code, by the name
        Model references use, from the Runs this connection opens; a name taken by a built-in, a declared provider or a
        Plugin raises `SettingInvalidError` (ADR 0101). `fetch_backends`, `proxy_providers` and `sinks` are the
        program's own Plugins, SDK handlers (ADR 0104). All of them run on the connection's thread pool.
        """
    def explain_settings(self, keys: Sequence[SettingKey] = ..., *, timeout: Timeout = ...) -> list[ExplainedSetting]: ...
    def start_crawl(
        self,
        crawl: Crawl,
        *,
        seeds: Sequence[Seed[Any] | SitemapSeed],
        run_id: str | None = None,
        settings: CrawlRunSettings = ...,
        timeout: Timeout = ...,
        limits: Limits | None = None,
        plan: str | PathLike[str] | None = None,
        recording: Recording | None = None,
    ) -> CrawlRun:
        """`AsyncFoxhound.start_crawl` in the sync API."""
    def open_crawl_run(self, run_id: str, *, timeout: Timeout = ...) -> CrawlRunView: ...
