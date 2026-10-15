"""The Run in code (ADR 0102, decided in #478).

A Run starts with `collie.start()` or `foxhound.start_crawl(crawl, …)`, which return once it is started. Its ID, chosen
by the program or generated, names it for good: a start whose ID names an existing Run resumes it, returns it ended, or
refuses, and nothing is inferred from a storage folder. `finish()` lets the Run finish and returns its report, `stop()`
ends it early with its report, and ADR 0078's `close()` is unchanged. Progress is a telemetry sequence (ADRs 0024,
0081); a crawl's outcomes are read from its committed state.
"""

from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from datetime import datetime
from os import PathLike
from types import TracebackType
from typing import Generic, Literal, Self, TypeAlias, TypeVar, final

from goodfetch._recording import RecordingReport
from goodfetch._concurrency import AsyncEventIterator, EventIterator
from goodfetch._models import RunUsage
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
from goodfetch._time import Timeout

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
    by: Literal["program", "person", "schedule"] | str | None
    """Who aborted it, for `abort`: its program's own `stop()` or `close()`, a person in the Console, or ADR 0036's
    scheduler."""

@final
@dataclass(frozen=True, kw_only=True)
class WaitingPause:
    """A Human pause waiting for a person (ADR 0092)."""

    session: str
    reason: str | None
    """The program's `reason`, or the engine's own, such as a login on a domain."""
    raised_by: Literal["program", "engine"] | str
    """`program`: `page.pause()`; `engine`: a pause the engine raised, such as ADR 0006's login recovery."""
    since: datetime

@dataclass(frozen=True, kw_only=True)
class RunProgress:
    """A Run's progress: a telemetry value, coalesced under load (ADR 0024)."""

    lifecycle: Lifecycle
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

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRunProgress(RunProgress):
    """A crawl's progress: what is in flight, what waits and why."""

    requests: CrawlRequestCounts
    waiting: Sequence[WaitGroup]
    in_flight: Sequence[InFlightRequest]
    concurrency: CrawlConcurrency

WeakenedSetting: TypeAlias = LimitSource
"""A setting that weakened a safeguard for the Run (ADR 0028), with its key, value, layer and origin: a costly control
switched off (ADR 0023), the Run trace's "no trace" mark (ADR 0031) included, or a value pressing a site harder than its
built-in default (ADR 0086)."""

@final
@dataclass(frozen=True, kw_only=True)
class Checkpoint:
    """A Checkpoint, as ADR 0002 reports it."""

    path: str
    size: int
    """In bytes."""
    digest: str
    epoch: int
    journal_position: int
    taken_at: datetime

@final
@dataclass(frozen=True, kw_only=True)
class NothingDone:
    """Why a Run did nothing (G1).

    - `already_ended`: the start found the Run ended, at `ended_at`;
    - `no_seed_handled`: no seed reached a handler, each refused when added or skipped when fetched, counted in
      `by_reason`;
    - `nothing_pending`: a resume or a retry found nothing left.
    """

    reason: Literal["already_ended", "no_seed_handled", "nothing_pending"] | str
    ended_at: datetime | None
    by_reason: Mapping[SkipReason, int]

@dataclass(frozen=True, kw_only=True)
class RunReport:
    """A Run report: what a Run states about itself when it ends."""

    id: str
    lifecycle: Lifecycle
    """`finished` or `stopped`: a `failed` Run raises instead."""
    stop_reason: RunStopReason | None
    """Set when `lifecycle` is `stopped`."""
    verdict: Verdict
    verdict_reasons: Sequence[str]
    """Why it is not `verified`: failed, missing or switched-off checks, uncertain Effects (ADR 0079)."""
    completeness: Completeness
    completeness_reasons: Sequence[str]
    """Why it is not `complete`: stopped early, a bound reached, offset pagination, pages left at `payment_required`
    (ADR 0005)."""
    usage: RunUsage
    """The final figures (ADR 0101), the uncapped mark included (ADR 0041)."""
    weakened: Sequence[WeakenedSetting]
    checkpoint: Checkpoint | None
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
    """A crawl's requests when its Run ended. Records per Dataset are #479's to add."""

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

    status: Literal["complete", "complete_with_rejections", "behind", "failed"] | str
    dead_letters: int

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRunReport(RunReport):
    """A crawl's Run report."""

    requests: CrawlRequestTotals
    records: Mapping[str, DatasetTotals]
    """Per Dataset name."""
    sinks: Mapping[str, SinkDeliveryStatus]
    """Per Sink name."""

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
        and returns its report. Resumable by its ID."""
    def progress(self) -> AsyncEventIterator[_Progress]:
        """The Run's progress: its first value at once, then the latest after each change, until the Run ends. Leave the
        loop to read one."""
    async def set_progress_message(self, message: str, *, timeout: Timeout = ...) -> None:
        """The program's own status line, shown in `progress()` and the Console."""
    async def checkpoint(self, *, path: str | PathLike[str] | None = None, timeout: Timeout = ...) -> Checkpoint:
        """Writes a Checkpoint now (ADR 0002), to `path`, or the Run's own folder when left out (ADR 0031)."""

class RunControl(Generic[_Report, _Progress]):
    """`AsyncRunControl` in the sync API."""

    @property
    def started_as(self) -> StartedAs: ...
    def finish(self, *, timeout: Timeout = ...) -> _Report: ...
    def stop(self, *, timeout: Timeout = ...) -> _Report: ...
    def progress(self) -> EventIterator[_Progress]: ...
    def set_progress_message(self, message: str, *, timeout: Timeout = ...) -> None: ...
    def checkpoint(self, *, path: str | PathLike[str] | None = None, timeout: Timeout = ...) -> Checkpoint: ...

class AsyncRunView(Generic[_Report, _Progress]):
    """A Run read by a program that does not drive it, opened by its ID with `open_run` (ADR 0102): a detached Run,
    another program's, or one that has ended. It never gives the Run work and takes no writer lock (ADR 0031); closing it
    frees only the view."""

    @property
    def id(self) -> str: ...
    def progress(self) -> AsyncEventIterator[_Progress]: ...
    async def report(self, *, timeout: Timeout = ...) -> _Report:
        """The Run's report: at once for an ended Run, at its end for a live one, which it never ends."""
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
    def close(self) -> None: ...
    def __enter__(self) -> Self: ...
    def __exit__(
        self, exc_type: type[BaseException] | None, exc: BaseException | None, tb: TracebackType | None
    ) -> None: ...

# The Run's Error codes (ADR 0102).

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
    """The Run is live in another program, or already open in this one: its writer's lock is held (ADR 0031)."""

    @property
    def details(self) -> RunInUseDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class LimitRaiseRefusedDetails:
    limit: str

@final
class LimitRaiseRefusedError(RefusedError):
    """A resume gave a Limit above the value the Run began with: only a person raises one (ADR 0041)."""

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
