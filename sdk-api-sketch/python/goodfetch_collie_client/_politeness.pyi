"""The Politeness and Site preferences Collie carries (ADRs 0086, 0088): the per-proxy limits, which protect the user's
own proxy accounts and hold a program's own navigation through them, how far they are shared, and whether AI-use signals
apply. Politeness toward sites (per domain, backoff, robots.txt, Crawl-delay) is a crawl's, in `goodfetch_foxhound`.
"""

from typing import Literal, TypeAlias, TypedDict

class ProxyPolitenessSettings(TypedDict, total=False):
    """`politeness.proxy.*`: per proxy endpoint and credential (ADR 0053), for a crawl and a program's own
    navigation through a proxy alike (ADR 0088)."""

    concurrency: int | None
    """Requests in flight through one proxy. `None`, the built-in: only the ceiling learned from its refusals."""
    rate: float | None
    """Requests per second through one proxy. `None`, the built-in: no rate."""

PolitenessSharing: TypeAlias = Literal["shared", "per_daemon", "run"]
"""How far a run shares Politeness (ADR 0053). Anything but `shared` is marked in the run's result."""

class PolitenessSettings(TypedDict, total=False):
    """`politeness.*` as Collie carries it."""

    proxy: ProxyPolitenessSettings
    sharing: PolitenessSharing
    """Built-in: `shared`."""

class SitePreferencesSettings(TypedDict, total=False):
    """`site_preferences.*` as a Collie Run sets it: Politeness and robots.txt are a crawl's (ADR 0084)."""

    respect_use_signals: bool
    """Apply the AI-use signals that concern the run's Purpose (ADR 0016). Built-in: `False`. A Collie program's own page
    is never refused for one (ADR 0085)."""
