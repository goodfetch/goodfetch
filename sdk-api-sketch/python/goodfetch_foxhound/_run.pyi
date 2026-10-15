"""A crawl's Run in code (ADR 0102, decided in #478): what its progress and its report add to every Run's, which are the
Collie client package's. `CrawlRun` itself is in `_connection`."""

from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from datetime import datetime
from typing import Literal, TypeAlias, final

from goodfetch_collie_client import BlockCause, LimitSource, RunProgress, RunReport, UncertainEffect
from goodfetch_foxhound._crawl import NotAddedReason
from goodfetch_foxhound._outcomes import CrawlRequestRef, SkipReason

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

@final
@dataclass(frozen=True, kw_only=True)
class LearnedRule:
    """A URL rule the Frontier learned, with its evidence (ADR 0005)."""

    kind: Literal["no_effect_parameter", "equivalent_form", "trap_path", "cursor_pagination"] | str
    """`equivalent_form`: `www.` and the bare host, `http` and `https`, a trailing slash or an index file shown to be one
    resource by a redirect, page signatures or `rel=canonical` (ADR 0120)."""
    domain: str
    rule: str
    evidence: str
    urls_merged: int

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

    uncertain_effects: Sequence[CrawlUncertainEffect]
    requests: CrawlRequestTotals
    records: Mapping[str, DatasetTotals]
    """Per Dataset name."""
    sinks: Mapping[str, SinkDeliveryStatus]
    """Per Sink name."""
    learned_rules: Sequence[LearnedRule]
    sitemaps: Mapping[str, SitemapTotals]
    """Per sitemap seed's URL, as the progress has them."""

@final
@dataclass(frozen=True, kw_only=True)
class CrawlUncertainEffect(UncertainEffect):
    """An uncertain Effect of a crawl's Run: one declared in a handler or a Hook sits in its crawl request's namespace
    (ADR 0106), a field only Foxhound fills, so it is Foxhound's type (ADR 0099)."""

    request: CrawlRequestRef | None
    """The crawl request whose namespace holds it; `None` for one the program declared outside a handler."""
