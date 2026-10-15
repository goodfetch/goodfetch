"""A crawl request's outcomes and Foxhound's own error of a handler (ADRs 0079, 0093). The error groups and every class
of Collie's codes are the Collie client package's (ADR 0095)."""

from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from datetime import datetime
from typing import Literal, TypeAlias, final

from goodfetch_collie_client._results import KnownErrorCode as CollieKnownErrorCode
from goodfetch_collie_client import BlockCause, GoodfetchError, HandlerFailedDetails, ProgramError

@final
class HandlerFailedError(ProgramError):
    """A crawl handler raised something other than a `GoodfetchError`; `__cause__` holds it in the process that raised it."""

    @property
    def details(self) -> HandlerFailedDetails: ...

SkipReason: TypeAlias = (
    Literal[
        "out_of_scope",
        "domain_policy",
        "duplicate",
        "robots_txt",
        "crawl_delay",
        "use_signals",
        "hook",
        "handler",
        "model_allowance",
    ]
    | str
)
@final
@dataclass(frozen=True, kw_only=True)
class CrawlRequestRef:
    """Which crawl request an outcome is about. Outcomes span Labels, so the Payload is not typed here."""

    id: str
    url: str
    label: str
    """The Label's name."""
    depth: int

@final
@dataclass(frozen=True, kw_only=True)
class RecordCounts:
    """What a request's commit did to its Records in one Dataset (ADR 0103)."""

    committed: int
    dropped: int
    """Dropped with a reason by a "Record before commit" Hook: counted, never stored."""
    rejected: int
    """In the Dataset's rejection table, read with `run.rejected_records()`."""

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRequestSucceeded:
    request: CrawlRequestRef
    ended_at: datetime
    records: Mapping[str, RecordCounts]
    """Per Dataset name, what its commit did to the Records it pushed (ADR 0103); empty when it pushed none."""
    attempts: int
    """Retries are not an outcome: every outcome counts its attempts and keeps the failed ones."""
    attempt_errors: Sequence[GoodfetchError]

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRequestFailed:
    request: CrawlRequestRef
    ended_at: datetime
    error: GoodfetchError
    attempts: int
    attempt_errors: Sequence[GoodfetchError]

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRequestSkipped:
    request: CrawlRequestRef
    ended_at: datetime
    reason: SkipReason
    message: str | None
    attempts: int
    attempt_errors: Sequence[GoodfetchError]

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRequestBlocked:
    request: CrawlRequestRef
    ended_at: datetime
    block_cause: BlockCause
    attempts: int
    attempt_errors: Sequence[GoodfetchError]

@final
@dataclass(frozen=True, kw_only=True)
class CrawlRequestOutcomeUnknown:
    """A kind a newer engine sent that this SDK does not know, kept as received (G3)."""

    request: CrawlRequestRef
    ended_at: datetime
    raw: str
    attempts: int
    attempt_errors: Sequence[GoodfetchError]

CrawlRequestOutcome: TypeAlias = (
    CrawlRequestSucceeded | CrawlRequestFailed | CrawlRequestSkipped | CrawlRequestBlocked | CrawlRequestOutcomeUnknown
)

KnownErrorCode: TypeAlias = CollieKnownErrorCode | Literal[
    "handler_failed",
    "hook_failed",
    "header_forbidden",
    "hook_ineffective",
    "hook_duplicate",
    "fetch_handler_failed",
    "sink_handler_failed",
    "payload_invalid",
    "payload_schema_changed",
    "label_duplicate",
    "label_unhandled",
    "label_unknown",
    "attempt_ended",
    "browser_required",
    "responses_not_kept",
    "retry_requested",
    "response_not_ok",
    "response_unavailable",
    "handler_lost",
    "site_preferences_refused",
    "hook_unknown",
    "dataset_duplicate",
    "dataset_unknown",
    "dataset_key_invalid",
    "record_invalid",
]
"""The Error codes this sketch shows: Collie's, which every Foxhound engine also sends, and Foxhound's own. Their classes
extend the client package's group classes, so a Collie program that meets one gets its group's class with the code kept
(ADRs 0079, 0099)."""

ErrorCode: TypeAlias = KnownErrorCode | str
"""A code a newer engine sends and this SDK does not know is kept as received (G3)."""
