"""The Run in code (ADR 0102, decided in #478).

A Run starts with `collie.start()` or `foxhound.start_crawl(crawl, …)`, which return once it is started. Its ID, chosen
by the program or generated, names it for good: a start whose ID names an existing Run resumes it, returns it ended, or
refuses, and nothing is inferred from a storage folder. `finish()` lets the Run finish and returns its report, `stop()`
ends it early with its report, and ADR 0078's `close()` is unchanged. Progress is a telemetry sequence (ADRs 0024,
0081); a crawl's outcomes are read from its committed state.
"""

from collections.abc import AsyncIterator, Iterator, Mapping, Sequence
from dataclasses import dataclass
from datetime import datetime
from os import PathLike
from types import TracebackType
from typing import Generic, Literal, Self, TypeAlias, TypeVar, final, overload

from goodfetch._ai import PlanReport
from goodfetch._recording import RecordingReport
from goodfetch._concurrency import AsyncEventIterator, EventIterator
from goodfetch._crawl import NotAddedReason
from goodfetch._data import JsonValue
from goodfetch._effects import UncertainEffect
from goodfetch._models import RunUsage
from goodfetch._output import StoredBytes
from goodfetch._results import (
    BlockCause,
    CrawlRequestRef,
    LimitSource,
    ProgramError,
    RefusedError,
    SkipReason,
    UnavailableError,
    Verdict,
)
from goodfetch._settings import ExplainedSetting
from goodfetch._time import Timeout

_T = TypeVar("_T")

StartedAs: TypeAlias = Literal["new", "resumed", "ended"] | str
"""How a start opened its Run: `new`; `resumed` from its committed state, the run file on this machine or an imported
Checkpoint (ADR 0002); or `ended`, an ID naming a Run that had already finished, returned as it is with nothing run."""

Lifecycle: TypeAlias = Literal["running", "finished", "stopped", "failed"] | str
"""Where a Run is (ADR 0029). A Run that ends `failed` makes `finish()` and `stop()` raise its error."""

Completeness: TypeAlias = Literal["complete", "incomplete", "unverified"] | str
"""Whether a Run's results cover its scope (ADRs 0005, 0029). A Collie Run its program finishes is `complete`."""

@final
@dataclass(frozen=True, kw_only=True)
class RunStopReason:
    """Why a Run stopped: ADR 0029's causes, and its own timeout (ADR 0080)."""

    kind: Literal["limit", "timeout", "abort", "engine_lost", "resources"] | str
    limit: str | None
    """The Limit by name, for `limit`."""
    limit_scope: str | None
    """For `limit`: its scope (a `LimitScope` value)."""
    limit_scope_id: str | None
    """For `limit`: the scope's ID."""
    resets_at: datetime | None
    """For `limit`: when its period resets; `None` when it has none."""
    by: Literal["program", "person", "schedule"] | str | None
    """Who aborted it, for `abort`: its program's own `stop()` or `close()`, a person in the Console, or ADR 0036's
    scheduler."""

@final
@dataclass(frozen=True, kw_only=True)
class WaitingPause:
    """A Human pause waiting for a person (ADR 0092).

    It begins and ends as a change of `progress()`; the person learns of it from ADR 0027's Notification and the
    Console's "Needs you". Its time counts toward no timeout, the Run's included, while its Usage counts toward Limits.
    """

    session: str
    page: str | None
    """The page it holds, by the page's ID, when it holds one."""
    reason: str | None
    """The program's `reason`, or the engine's own, such as a login on a domain."""
    until: str | None
    """The `until` condition as text, the Locator's or the URL matcher's description; `None`: a person's "Done" ends it."""
    raised_by: Literal["program", "engine"] | str
    """`program`: `page.pause()`; `engine`: a pause the engine raised."""
    step: Literal["login_recovery", "challenge"] | str | None
    """For an engine's pause, the step that raised it: ADR 0006's login recovery or ADR 0035's Challenge ladder."""
    since: datetime
    deadline: datetime | None
    """When it runs out: the first of `timeout.human_pause` and, in a scheduled run, ADR 0036's pause timeout; `None`
    when it waits without limit."""

@dataclass(frozen=True, kw_only=True)
class RunProgress:
    """A Run's progress: a telemetry value, coalesced under load (ADR 0024)."""

    lifecycle: Lifecycle
    writer: Literal["live", "none"] | str
    """`none` when nothing works the Run, as after a crash, until a start with its ID resumes it."""
    epoch: int
    """The run epoch (ADR 0003), which a resume or a migration to another machine advances."""
    checkpoint_at: datetime | None
    """When the last Checkpoint was written."""
    pauses: Sequence[WaitingPause]
    message: str | None
    """The program's own line, set with `set_progress_message` (ADR 0037's RT-15)."""
    at: datetime
    """When the engine took this value."""

WaitReason: TypeAlias = (
    Literal[
        "domain_concurrency",
        "domain_delay",
        "crawl_delay",
        "backoff",
        "robots_txt",
        "handler_concurrency",
        "governor",
        "session",
        "human_pause",
    ]
    | str
)
"""Why a due crawl request is held back; a reason a newer engine sends is kept as received (G3).

- `domain_concurrency`, `domain_delay`: the domain's slots and delay (ADR 0086);
- `crawl_delay`: a host's Crawl-delay, which raised its domain's delay;
- `backoff`: after a 429, 502, 503, 504, or a connection refused or timed out (ADRs 0021, 0086);
- `robots_txt`: the origin's robots.txt, not served yet;
- `handler_concurrency`: `crawl.handler.concurrency` handlers already running in the program;
- `governor`: ADR 0040's resource governor, admitting nothing new under memory pressure;
- `session`: waiting for a Session of the pool, such as one logging in;
- `human_pause`: its Session waits for a person.
"""

@final
@dataclass(frozen=True, kw_only=True)
class WaitingDomain:
    domain: str
    requests: int
    until: datetime | None
    """When it is next admitted; `None`: once something else happens, such as a person ending a pause."""

@final
@dataclass(frozen=True, kw_only=True)
class WaitGroup:
    """The due requests one reason holds back."""

    reason: WaitReason
    requests: int
    domains: Sequence[WaitingDomain]
    """The busiest first."""
    more_domains: int
    """Domains held for this reason beyond those listed."""
    setting: LimitSource | None
    """The setting behind the wait, with its value and layer, where one sets it."""

@final
@dataclass(frozen=True, kw_only=True)
class InFlightRequest:
    request: CrawlRequestRef
    phase: Literal["fetching", "handling"] | str
    """`fetching`: from its start until its response arrives (ADR 0086); `handling`: its handler runs in the program."""
    tier: str
    """The tier fetching it, as reported (ADR 0093's `ReportedFetchTier`)."""
    attempt: int
    since: datetime
    waiting: PermitWait | None
    """While its handler's own navigation or fetch waits for a permit, which counts against no timeout (ADR 0093)."""

@final
@dataclass(frozen=True, kw_only=True)
class PermitWait:
    """Why a handler's own navigation or fetch waits, and the setting behind it."""

    reason: WaitReason
    setting: LimitSource | None

@final
@dataclass(frozen=True, kw_only=True)
class UnreadSitemap:
    """A sitemap the engine could not read."""

    url: str
    reason: Literal["fetch_failed", "robots_txt", "unparsable"] | str
    """`fetch_failed`, with the response status when one came; `robots_txt`, which disallows it; `unparsable`."""
    status: int | None

@final
@dataclass(frozen=True, kw_only=True)
class SitemapTotals:
    """A sitemap seed's sitemaps, read while the Run works (ADR 0093's R3): its entries counted, never listed one by
    one."""

    sitemaps_read: int
    sitemaps_unread: Sequence[UnreadSitemap]
    """At most a fixed number, the rest counted in `more_sitemaps_unread`."""
    more_sitemaps_unread: int
    entries_added: int
    entries_not_added: Mapping[NotAddedReason, int]
    """The entries not added, by reason: `no_rule` for one that matched no rule, and an enqueue's other reasons (ADR
    0093)."""

@final
@dataclass(frozen=True, kw_only=True)
class HandlerConcurrency:
    running: int
    limit: LimitSource
    """`crawl.handler.concurrency`, with its value and layer."""

@final
@dataclass(frozen=True, kw_only=True)
class GovernorState:
    """ADR 0040's resource governor."""

    level: Literal["normal", "pressure", "critical"] | str
    open_pages: int
    allowed_pages: int
    allowance_from: Literal["cgroup", "job_object", "memory_allowance", "physical_memory"] | str
    """What set the memory allowance: the process's cgroup, a Windows Job object, `memory_allowance`, or half of
    physical memory."""

@final
@dataclass(frozen=True, kw_only=True)
class CrawlConcurrency:
    """Why the crawl runs as many requests at once as it does (P47)."""

    handlers: HandlerConcurrency
    governor: GovernorState

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRequestCounts:
    """A crawl's requests by state: live ones, then ended ones by outcome."""

    pending: int
    fetching: int
    handling: int
    paused: int
    succeeded: int
    failed: int
    skipped: int
    blocked: int
    by_skip_reason: Mapping[SkipReason, int]
    by_error_code: Mapping[str, int]
    by_block_cause: Mapping[BlockCause, int]

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRunProgress(RunProgress):
    """A crawl's progress: what is in flight, what waits and why."""

    requests: CrawlRequestCounts
    waiting: Sequence[WaitGroup]
    in_flight: Sequence[InFlightRequest]
    concurrency: CrawlConcurrency
    idle: bool
    """`True` while `crawl.keep_alive` holds the Run `running` with an empty Frontier and nothing in flight, until
    `add_seed` gives it work; idle time counts toward the Run's own timeout."""
    sitemaps: Mapping[str, SitemapTotals]
    """Per sitemap seed's URL."""

WeakenedSetting: TypeAlias = LimitSource
"""A setting that weakened a safeguard for the Run (ADR 0028), with its key, value, layer and origin: a costly control
switched off (ADR 0023), the Run trace's "no trace" mark (ADR 0031) included, or a value pressing a site harder than its
built-in default (ADR 0086)."""

@dataclass(frozen=True, kw_only=True)
class CheckpointInfo:
    """A Checkpoint, as ADR 0002 reports it, written where the engine keeps the Run's: its own folder (ADR 0031), or
    wherever Goodfetch Cloud stores them."""

    location: str
    """Where the engine wrote it, on the engine's side."""
    size: int
    """In bytes."""
    digest: str
    epoch: int
    journal_position: int
    taken_at: datetime

@final
class AsyncCheckpoint(CheckpointInfo):
    """A Checkpoint just written, copied to this machine as ADR 0103's Export is."""

    async def save_as(self, path: str | PathLike[str], *, timeout: Timeout = ...) -> None:
        """Copies it to a file on this machine, wherever the engine runs."""
    def stream(self) -> AsyncIterator[bytes]: ...

@final
class Checkpoint(CheckpointInfo):
    """`AsyncCheckpoint` in the sync API."""

    def save_as(self, path: str | PathLike[str], *, timeout: Timeout = ...) -> None: ...
    def stream(self) -> Iterator[bytes]: ...

@final
@dataclass(frozen=True, kw_only=True)
class PluginRun:
    """A Plugin that ran in the Run (ADRs 0016, 0033)."""

    name: str
    version: str
    network_level: str
    missing_confinement: Sequence[str]
    """Where Operator policy allowed reduced confinement for a first-party Plugin, the parts the host could not provide."""

@final
@dataclass(frozen=True, kw_only=True)
class RunWarning:
    """A warning the Run carries: a `stealth` choice (ADR 0035), stale Clean-up rules (ADR 0047), throttled CPU (ADR
    0049), a degraded Politeness store (ADR 0053), a Plugin's reported figures (ADR 0033). A newer engine's code is kept."""

    code: str
    message: str

@final
@dataclass(frozen=True, kw_only=True)
class LearnedRule:
    """A URL rule the Frontier learned, with its evidence (ADR 0005)."""

    kind: Literal["no_effect_parameter", "trap_path", "cursor_pagination"] | str
    domain: str
    rule: str
    evidence: str
    urls_merged: int

@final
@dataclass(frozen=True, kw_only=True)
class VerdictReason:
    """Why a Run is not `verified`, naming the AI task concerned, an Agent's included, by its `call_id` (ADR 0100)."""

    code: str
    """Such as `success_check_failed`, `no_success_check`, `check_switched_off` or `effect_uncertain`; a newer engine's
    is kept (G3)."""
    message: str
    call_id: str | None

class AsyncRunTrace:
    """The Run trace (ADR 0031), copied to this machine as an Export is (ADR 0103): what was recorded so far, in the
    format ADR 0031 exports it in. A person replays it in the Console at `console_url`."""

    @property
    def size(self) -> int:
        """In bytes."""
    async def save_as(self, path: str | PathLike[str], *, timeout: Timeout = ...) -> None:
        """Copies it to a file on this machine, wherever the engine runs."""
    def stream(self) -> AsyncIterator[bytes]: ...

class RunTrace:
    """`AsyncRunTrace` in the sync API."""

    @property
    def size(self) -> int: ...
    def save_as(self, path: str | PathLike[str], *, timeout: Timeout = ...) -> None: ...
    def stream(self) -> Iterator[bytes]: ...

@final
@dataclass(frozen=True, kw_only=True)
class NothingDone:
    """Why a Run did nothing (G1).

    - `already_ended`: the start found the Run ended, at `ended_at`;
    - `no_seed_handled`: no seed reached a handler, each refused when added, counted in `refused` by an enqueue's
      reason, or skipped when fetched, counted in `skipped` by a skip reason;
    - `nothing_pending`: a resume or a retry found nothing left.
    """

    reason: Literal["already_ended", "no_seed_handled", "nothing_pending"] | str
    ended_at: datetime | None
    refused: Mapping[NotAddedReason, int]
    skipped: Mapping[SkipReason, int]

@dataclass(frozen=True, kw_only=True)
class RunReport:
    """A Run report: what a Run states about itself when it ends."""

    id: str
    lifecycle: Lifecycle
    """`finished` or `stopped`: a `failed` Run raises instead."""
    stop_reason: RunStopReason | None
    """Set when `lifecycle` is `stopped`."""
    verdict: Verdict
    """The Run's Verdict (ADR 0029), reading every check it relies on, each AI task's Success check included."""
    verdict_reasons: Sequence[VerdictReason]
    """Why it is not `verified`: failed, missing or switched-off checks, uncertain Effects (ADR 0079)."""
    uncertain_effects: Sequence[UncertainEffect]
    """Each Effect left uncertain, waiting for a person to reconcile it in the Console (ADRs 0029, 0106)."""
    completeness: Completeness
    completeness_reasons: Sequence[str]
    """Why it is not `complete`: stopped early, a bound reached, offset pagination, pages left at `payment_required`
    (ADR 0005)."""
    usage: RunUsage
    """The final figures (ADR 0101), the uncapped mark included (ADR 0041)."""
    plan: PlanReport
    """What became of the Run's Plan (ADR 0091): every Run has one, its file `None` when it named none. Each Agent task is
    one entry (ADR 0100)."""
    policy_version: str | None
    """The Operator policy version it ran under (ADR 0031)."""
    settings: Sequence[ExplainedSetting]
    """Each effective value, with the layer that set it and the one that locked it (ADR 0028)."""
    plugins: Sequence[PluginRun]
    warnings: Sequence[RunWarning]
    weakened: Sequence[WeakenedSetting]
    checkpoint: CheckpointInfo | None
    """The last Checkpoint written."""
    nothing_done: NothingDone | None
    console_url: str | None
    """The Run's page in the Console, where its Run trace replays, when the engine knows a Console address (ADR 0031)."""
    recording: RecordingReport | None
    """What the Run did with its Response recording (ADR 0105): `None` when it named none. A replayed Run says so here."""
    started_at: datetime
    ended_at: datetime

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRequestTotals:
    """A crawl's requests when its Run ended; its Records per Dataset are the report's `records`."""

    succeeded: int
    failed: int
    skipped: int
    blocked: int
    pending: int
    """Left in the Frontier by a Run that stopped."""
    by_skip_reason: Mapping[SkipReason, int]
    by_error_code: Mapping[str, int]
    by_block_cause: Mapping[BlockCause, int]

@final
@dataclass(frozen=True, kw_only=True)
class DatasetTotals:
    """A Dataset's Records over the Run (ADR 0103)."""

    committed: int
    dropped: int
    rejected: int
    dropped_by_reason: Mapping[str, int]
    """The dropped ones, by the Hook's reason."""

@final
@dataclass(frozen=True, kw_only=True)
class SinkDeliveryStatus:
    """A Sink's delivery (ADR 0026), as it stood when the report was read: a Sink may go on delivering after the Run
    ends."""

    status: Literal["complete", "complete_with_rejections", "behind", "failed", "inactive"] | str
    """ADR 0026's, or `inactive` for a Sink inherited from the connection or configuration that names none of the
    crawl's Datasets, which delivers nothing (ADR 0103)."""
    undeclared_datasets: Sequence[str]
    """For an `inactive` Sink, the Dataset names it gave that the crawl does not declare."""
    guarantee_class: str
    """ADR 0026's Guarantee class."""
    dead_letters: int
    at_least_once: int
    """Deliveries that fell outside the destination's deduplication window, so were at-least-once."""

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRunReport(RunReport):
    """A crawl's Run report."""

    requests: CrawlRequestTotals
    records: Mapping[str, DatasetTotals]
    """Per Dataset name."""
    sinks: Mapping[str, SinkDeliveryStatus]
    """Per Sink name."""
    learned_rules: Sequence[LearnedRule]
    sitemaps: Mapping[str, SitemapTotals]
    """Per sitemap seed's URL, as the progress has them."""

_Report = TypeVar("_Report", bound=RunReport, covariant=True)
_Progress = TypeVar("_Progress", bound=RunProgress, covariant=True)

class AsyncRunControl(Generic[_Report, _Progress]):
    """What every Run does besides its pages: end, report, show progress and write Checkpoints."""

    @property
    def started_as(self) -> StartedAs:
        """How the start opened this Run. On `ended`, a call that would give it new work raises `RunEndedError`."""
    async def finish(self, *, timeout: Timeout = ...) -> _Report:
        """Lets the Run finish, ends it `finished`, closes its Sessions and pages, and returns its report.

        At once for a Collie Run, whose program says it is done; once its Frontier is empty and nothing is in flight for
        a crawl, or, under `crawl.keep_alive`, once drained after this call; when the engine ends it for a detached Run.
        If something stops the Run first (a Limit, its timeout, a person, the engine), the report says `stopped`. Its
        wait has no timeout of its own: the Run's own bounds it, and a `timeout` given here, or cancelling the task,
        ends only the wait.
        """
    async def stop(self, *, timeout: Timeout = ...) -> _Report:
        """Stops the Run at its next safe point with a Checkpoint, `stopped` and `incomplete` as an abort by its program,
        and returns its report. Resumable by its ID. Its wait is bounded by `timeout.close`, as `close()`'s is, or by
        `timeout`; past it, it raises `DeadlineExceededError` naming the setting, and the Run still stops at its next safe
        point."""
    def progress(self) -> AsyncEventIterator[_Progress]:
        """The Run's progress: its first value at once, then the latest after each change, until the Run ends. Leave the
        loop to read one."""
    async def set_progress_message(self, message: str, *, timeout: Timeout = ...) -> None:
        """The program's own status line, shown in `progress()` and the Console."""
    async def checkpoint(self, *, timeout: Timeout = ...) -> AsyncCheckpoint:
        """Writes a Checkpoint now (ADR 0002), where the engine keeps the Run's."""
    async def trace(self, *, timeout: Timeout = ...) -> AsyncRunTrace | None:
        """The Run trace recorded so far; `None` when the Run keeps none, which the report's `weakened` shows."""

class RunControl(Generic[_Report, _Progress]):
    """`AsyncRunControl` in the sync API."""

    @property
    def started_as(self) -> StartedAs: ...
    def finish(self, *, timeout: Timeout = ...) -> _Report: ...
    def stop(self, *, timeout: Timeout = ...) -> _Report: ...
    def progress(self) -> EventIterator[_Progress]: ...
    def set_progress_message(self, message: str, *, timeout: Timeout = ...) -> None: ...
    def checkpoint(self, *, timeout: Timeout = ...) -> Checkpoint: ...
    def trace(self, *, timeout: Timeout = ...) -> RunTrace | None: ...

class AsyncKeyValueStoreReader:
    """A Key-value store's reads, for a Run view: `AsyncKeyValueStore` without its writes."""

    @overload
    async def get(self, key: str, *, timeout: Timeout = ...) -> JsonValue | None: ...
    @overload
    async def get(self, key: str, schema: type[_T], *, timeout: Timeout = ...) -> _T | None: ...
    async def get_bytes(self, key: str, *, timeout: Timeout = ...) -> StoredBytes | None: ...
    def keys(self, *, prefix: str = "") -> AsyncEventIterator[str]: ...

class KeyValueStoreReader:
    """`AsyncKeyValueStoreReader` in the sync API."""

    @overload
    def get(self, key: str, *, timeout: Timeout = ...) -> JsonValue | None: ...
    @overload
    def get(self, key: str, schema: type[_T], *, timeout: Timeout = ...) -> _T | None: ...
    def get_bytes(self, key: str, *, timeout: Timeout = ...) -> StoredBytes | None: ...
    def keys(self, *, prefix: str = "") -> EventIterator[str]: ...

class AsyncRunView(Generic[_Report, _Progress]):
    """A Run read by a program that does not drive it, opened by its ID with `open_run` (ADR 0102): a detached Run,
    another program's, or one that has ended. It never gives the Run work and takes no writer lock (ADR 0031); closing it
    frees only the view."""

    @property
    def id(self) -> str: ...
    def progress(self) -> AsyncEventIterator[_Progress]:
        """Ends after what is committed when the Run has no live writer."""
    async def report(self, *, timeout: Timeout = ...) -> _Report:
        """The Run's report: at once for an ended Run, at its end for a live one, which it never ends. On a Run with no
        live writer that has not ended, raises `RunInterruptedError`."""
    async def trace(self, *, timeout: Timeout = ...) -> AsyncRunTrace | None:
        """The Run trace recorded so far; `None` when the Run keeps none."""
    @property
    def key_value_store(self) -> AsyncKeyValueStoreReader:
        """Its Key-value store's reads, on every Run (ADR 0103)."""
    async def close(self) -> None: ...
    async def __aenter__(self) -> Self: ...
    async def __aexit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

class RunView(Generic[_Report, _Progress]):
    """`AsyncRunView` in the sync API."""

    @property
    def id(self) -> str: ...
    def progress(self) -> EventIterator[_Progress]: ...
    def report(self, *, timeout: Timeout = ...) -> _Report: ...
    def trace(self, *, timeout: Timeout = ...) -> RunTrace | None: ...
    @property
    def key_value_store(self) -> KeyValueStoreReader: ...
    def close(self) -> None: ...
    def __enter__(self) -> Self: ...
    def __exit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

# The Run's Error codes (ADR 0102).

@final
class RunInterruptedError(UnavailableError):
    """A view's `report()` on a Run with no live writer that has not ended: a start with its ID resumes it."""

    @property
    def details(self) -> RunIdDetails: ...

@final
class RunNotFoundError(ProgramError):
    """The ID names no Run in the Run index."""

    @property
    def details(self) -> RunIdDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class RunIdDetails:
    id: str

@final
class RunIdInvalidError(ProgramError):
    """An ID the engine refuses, such as one with characters a folder name cannot hold."""

    @property
    def details(self) -> RunIdDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class RunInputDifference:
    """An input of a resumed Run that differs from what the Run began with."""

    input: Literal["product", "kind", "seeds", "plan", "caller"] | str
    recorded: str
    given: str

@final
@dataclass(frozen=True, kw_only=True)
class RunConflictDetails:
    id: str
    differences: Sequence[RunInputDifference]

@final
class RunConflictError(ProgramError):
    """The ID names a Run started with other inputs, or by another caller: give another ID."""

    @property
    def details(self) -> RunConflictDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class RunEndedDetails:
    id: str
    lifecycle: Lifecycle
    ended_at: datetime

@final
class RunEndedError(ProgramError):
    """A call that would give an ended Run new work, such as `new_page()` or `add_seed()` on a Run that `started_as`
    `ended`."""

    @property
    def details(self) -> RunEndedDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class RunInUseDetails:
    id: str
    owner: str

@final
class RunInUseError(RefusedError):
    """The Run is live in another program, or already open in this one: its writer's lock is held (ADR 0031). Retryable:
    that program may end."""

    @property
    def details(self) -> RunInUseDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class LimitRaiseRefusedDetails:
    limit: str

@final
class LimitRaiseRefusedError(RefusedError):
    """A resume gave a Limit above the Limit as it stands: only a person raises one (ADR 0041)."""

    @property
    def details(self) -> LimitRaiseRefusedDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class RunStateUnavailableDetails:
    id: str
    state: Literal["missing", "damaged"] | str
    path: str | None

@final
class RunStateUnavailableError(UnavailableError):
    """The Run index lists the Run, but its state is missing or unreadable: nothing starts afresh under its ID."""

    @property
    def details(self) -> RunStateUnavailableDetails: ...
