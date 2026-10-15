"""Crawl politeness defaults (ADR 0086, decided in #480).

With nothing set, a crawl obeys robots.txt (RFC 9309) and keeps one request in flight per domain, one second between
starts (G5). These settings govern the requests a crawl's Frontier admits and every navigation a crawl handler makes
from its page. Politeness is Foxhound's (ADR 0084): a program's own navigation outside a handler (`page.goto`, an AI
verb or an Agent in its Session) is Collie's, directed by the program like a browser's, and nothing of Politeness gates
or counts it; whether Collie carries the per-proxy limits is #455's. A value that lets a crawl press a site harder than
its built-in default is the program's explicit choice: the Audit log records it, the run's result lists it, and the run
logs it once as a warning through `logging`. An Operator policy or a Recipe may lock any of them (ADR 0028).
"""

from datetime import timedelta
from typing import TypedDict

from goodfetch_collie_client._politeness import PolitenessSettings as ColliePolitenessSettings
from goodfetch_collie_client._politeness import SitePreferencesSettings as CollieSitePreferencesSettings


class DomainPolitenessSettings(TypedDict, total=False):
    """`politeness.domain.*`: per domain, keyed as in ADR 0004, and shared by every run of the user (ADR 0053)."""

    concurrency: int
    """Requests in flight per domain, each from its start until its response (a browser-tier page until its load event),
    whatever its page loads. Built-in: 1. At 1, a crawl's Run that switches hedges or `fetch.tier_race` on is refused."""
    delay: timedelta
    """The least time between two request starts to one domain; zero for none. Built-in: one second. While robots.txt
    is respected, a host's Crawl-delay raises it for the host's whole domain key."""

class BackoffSettings(TypedDict, total=False):
    """`politeness.backoff.*`: on 429, 502, 503, 504, an HTTP/2 `ENHANCE_YOUR_CALM`, and a connection refused or timed
    out."""

    enabled: bool
    """Built-in: `True`. Off, the run records that it ignored them and `Retry-After` (ADR 0023)."""
    initial: timedelta
    """The least wait. Each wait is drawn from the `Entropy` handle between this and three times the previous one, at
    most `max`: decorrelated jitter, never below it (ADR 0118). Built-in: two seconds."""
    max: timedelta
    """The longest computed wait; a `Retry-After` may ask for more. Built-in: 60 seconds."""

class RetryAfterSettings(TypedDict, total=False):
    """`politeness.retry_after.*` (ADR 0118): a `Retry-After` in seconds or as an HTTP-date, on a 429, 502, 503, 504 or a
    3xx, holds its domain for the longer of it and the backoff. Nothing is sent to the domain before the site's time,
    whatever this is set to, so it presses no site harder."""

    max: timedelta
    """The longest `Retry-After` a domain is waited for. Past it, the requests the wait holds end failed with
    `response_unavailable`, its `retry_after` set, for `retry_failed()` later. Built-in: 15 minutes."""

class IPPolitenessSettings(TypedDict, total=False):
    """`politeness.ip.*`: keyed by the IP the engine resolves (ADR 0021)."""

    concurrency: int | None
    """Requests in flight per resolved IP. `None`, the built-in: no limit beyond the domain's."""

class PolitenessSettings(ColliePolitenessSettings, total=False):
    """`politeness.*`: one family, so `goodfetch-foxhound config explain politeness` shows every limit, Collie's
    per-proxy limits included."""

    domain: DomainPolitenessSettings
    backoff: BackoffSettings
    retry_after: RetryAfterSettings
    ip: IPPolitenessSettings

class SitePreferencesSettings(CollieSitePreferencesSettings, total=False):
    """`site_preferences.*`: what the engine does with the Site preferences it always reads and records in Provenance
    (ADR 0016)."""

    respect_robots_txt: bool
    """Obey robots.txt for crawl requests, its Crawl-delay included, start URLs too. Built-in: `True`."""
    max_crawl_delay: timedelta | None
    """The longest Crawl-delay a crawl waits out; a host asking for more has its requests skipped with `crawl_delay`.
    `None`: wait out any. Built-in: 30 seconds."""
    robots_txt_user_agent: str | None
    """The operator's own product token, as the crawl's user agent names it: its robots.txt groups apply, merged with
    `goodfetch-foxhound`'s (ADR 0090), before `*`. Built-in: none. Setting it is recorded."""
