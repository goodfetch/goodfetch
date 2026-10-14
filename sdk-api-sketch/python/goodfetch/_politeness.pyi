"""Crawl politeness defaults (ADR 0086, decided in #480).

With nothing set, a crawl obeys robots.txt (RFC 9309) and keeps one request in flight per domain, one second between
starts (G5). These settings govern the requests a crawl's Frontier admits and every navigation a crawl handler makes from
its page; a program's own navigation outside a handler (`page.goto`, an AI verb or an Agent in its Session) is directed by
the program, like a browser's, and only the per-proxy limits and `sharing` apply to it, though a 429 it receives still
backs off a crawl sharing the domain. A value that lets a crawl press a site harder than its built-in default is the program's explicit
choice: the Audit log records it, the run's result lists it, and the run logs it once as a warning through `logging`.
An Operator policy or a Recipe may lock any of them (ADR 0028).
"""

from datetime import timedelta
from typing import Literal, TypeAlias, TypedDict

class DomainPolitenessSettings(TypedDict, total=False):
    """`politeness.domain.*`: per domain, keyed as in ADR 0004, and shared by every run of the user (ADR 0053)."""

    concurrency: int
    """Requests in flight per domain, each from its start until its response (a browser-tier page until its load event),
    whatever its page loads. Built-in: 1. At 1, switching hedges or `tier_race` on is refused."""
    delay: timedelta
    """The least time between two request starts to one domain; zero for none. Built-in: one second. While robots.txt
    is respected, a host's Crawl-delay raises it for that host."""

class BackoffSettings(TypedDict, total=False):
    """`politeness.backoff.*`: on 429, 502, 503, 504, `Retry-After`, and a connection refused or timed out."""

    enabled: bool
    """Built-in: `True`. Off, the run records that it ignored them (ADR 0023)."""
    initial: timedelta
    """The first wait, doubled at each signal. Built-in: two seconds."""
    max: timedelta
    """The longest wait. Built-in: 60 seconds."""

class IpPolitenessSettings(TypedDict, total=False):
    """`politeness.ip.*`: keyed by the IP the engine resolves (ADR 0021)."""

    concurrency: int | None
    """Requests in flight per resolved IP. `None`, the built-in: no limit beyond the domain's."""

class ProxyPolitenessSettings(TypedDict, total=False):
    """`politeness.proxy.*`: per proxy endpoint and credential (ADR 0053); they apply to every run."""

    concurrency: int | None
    """Requests in flight through one proxy. `None`, the built-in: only the ceiling learned from its refusals."""
    rate: float | None
    """Requests per second through one proxy. `None`, the built-in: no rate."""

PolitenessSharing: TypeAlias = Literal["shared", "per_daemon", "run"]
"""How far a run shares Politeness (ADR 0053). Anything but `shared` is marked in the run's result."""

class PolitenessSettings(TypedDict, total=False):
    """`politeness.*`: one family, so `goodfetch config explain politeness` shows every limit."""

    domain: DomainPolitenessSettings
    backoff: BackoffSettings
    ip: IpPolitenessSettings
    proxy: ProxyPolitenessSettings
    sharing: PolitenessSharing
    """Built-in: `shared`."""

class SitePreferencesSettings(TypedDict, total=False):
    """`site_preferences.*`: what the engine does with the Site preferences it always reads and records in Provenance
    (ADR 0016)."""

    respect_robots_txt: bool
    """Obey robots.txt for crawl requests, its Crawl-delay included, start URLs too. Built-in: `True`."""
    max_crawl_delay: timedelta | None
    """The longest Crawl-delay a crawl waits out; a host asking for more has its requests skipped with `crawl_delay`.
    `None`: wait out any. Built-in: 30 seconds."""
    robots_txt_user_agent: str | None
    """The operator's own product token, as the crawl's user agent names it: its robots.txt groups apply, merged with
    `Foxhound`'s, before `*`. Built-in: none. Setting it is recorded."""
    respect_use_signals: bool
    """Apply the AI-use signals that concern the run's Purpose (ADR 0016). Built-in: `False`."""

class ColliePolitenessSettings(TypedDict, total=False):
    """The politeness a Collie Run can set: its program directs its own navigation, so no per-domain limit, backoff or
    robots.txt."""

    proxy: ProxyPolitenessSettings
    sharing: PolitenessSharing

class CollieSitePreferencesSettings(TypedDict, total=False):
    """The Site preferences a Collie Run can set."""

    respect_use_signals: bool
