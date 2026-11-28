"""A crawl's and a Label's settings (ADRs 0083 and 0093), checked by pyright and mypy.

Starting a crawl takes `CrawlSettings`, Collie's and Foxhound's Run keys,
and each of its Labels `LabelSettings`, the keys a Label sets alone:
nested `TypedDict` literals by the wire's `snake_case` segments, durations
as `timedelta`, `None` only where a setting can be cleared. A key outside
its scope, a misspelled or camelCase one and a wrong value are each
refused; mypy's `--strict` reports an ignore that no longer silences an
error. TypeScript's `settings.test-d.ts` of Foxhound's package is its twin.
"""

from __future__ import annotations

from datetime import timedelta
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from goodfetch_collie_client.generated.settings import (
        ConnectionPolitenessSettings as CollieConnectionPolitenessSettings,
    )
    from goodfetch_collie_client.generated.settings import (
        ConnectionSitePreferencesSettings as CollieConnectionSitePreferencesSettings,
    )
    from goodfetch_collie_client.generated.settings import (
        RunPolitenessSettings as CollieRunPolitenessSettings,
    )
    from goodfetch_collie_client.generated.settings import (
        RunSitePreferencesSettings as CollieRunSitePreferencesSettings,
    )
    from goodfetch_foxhound.generated.settings import (
        ConnectionPolitenessSettings,
        ConnectionSitePreferencesSettings,
        CrawlSettings,
        FoxhoundSettings,
        LabelSettings,
        RunPolitenessSettings,
        RunSitePreferencesSettings,
    )

SECOND = timedelta(seconds=1)


def sets_a_crawl_and_its_labels() -> None:
    """A crawl takes Collie's and Foxhound's Run keys, a Label its own."""
    crawl: CrawlSettings = {
        "fetch": {"tier": "auto", "http_from": "learned"},
        "politeness": {"domain": {"concurrency": 2}},
        "crawl": {"max_depth": 3, "keep_url_fragment": True},
        "timeout": {"navigation": timedelta(seconds=30), "handler": SECOND},
        "content": {"extractor": "trafilatura"},
    }
    label: LabelSettings = {
        "fetch": {"tier": "browser", "encoding": None},
        "crawl": {"keep_url_fragment": False},
        "timeout": {"hook": SECOND},
        "session": {"source": None},
    }
    connection: FoxhoundSettings = {"fetch": {"tier": "http"}}
    del crawl, label, connection


def refuses_what_a_label_cannot_set() -> None:
    """A key outside a Label's scope, a wrong key or a wrong value is a type error."""
    a: LabelSettings = {"politeness": {"domain": {"concurrency": 2}}}  # type: ignore[typeddict-unknown-key]
    b: LabelSettings = {"proxy": {"base_url": None}}  # type: ignore[typeddict-unknown-key]
    c: LabelSettings = {"crawl": {"max_depth": 3}}  # type: ignore[typeddict-unknown-key]
    d: LabelSettings = {"fetch": {"tierr": "http"}}  # type: ignore[typeddict-unknown-key]
    e: LabelSettings = {"fetch": {"httpFrom": "learned"}}  # type: ignore[typeddict-unknown-key]
    f: LabelSettings = {"fetch": {"tier": "fastest"}}  # type: ignore[typeddict-item]
    g: LabelSettings = {"timeout": {"hook": 10_000}}  # type: ignore[typeddict-item]
    h: CrawlSettings = {"fetch": {"tier": None}}  # type: ignore[typeddict-item]
    i: CrawlSettings = {"politeness": {"degraded_max": SECOND}}  # type: ignore[typeddict-unknown-key]
    j: FoxhoundSettings = {"politeness": {"degraded_max": SECOND}}  # type: ignore[typeddict-unknown-key]
    del a, b, c, d, e, f, g, h, i, j


def politeness_extends_collie(
    collie_conn_politeness: CollieConnectionPolitenessSettings,
    collie_run_politeness: CollieRunPolitenessSettings,
    collie_conn_site_prefs: CollieConnectionSitePreferencesSettings,
    collie_run_site_prefs: CollieRunSitePreferencesSettings,
) -> None:
    """Foxhound's Politeness and Site-preferences types extend Collie's (ADR 0086)."""
    conn_politeness: ConnectionPolitenessSettings = {
        "sharing": collie_conn_politeness.get("sharing", "shared"),
        "delegation": collie_conn_politeness.get("delegation", True),
        "domain": {"concurrency": 2, "delay": SECOND},
        "backoff": {"enabled": True, "initial": SECOND, "max": timedelta(seconds=60)},
        "ip": {"concurrency": 4},
        "retry_after": {"max": timedelta(minutes=15)},
    }
    run_politeness: RunPolitenessSettings = {
        "sharing": collie_run_politeness.get("sharing", "shared"),
        "delegation": collie_run_politeness.get("delegation", True),
        "domain": {"concurrency": 2, "delay": SECOND},
        "backoff": {"enabled": True, "initial": SECOND, "max": timedelta(seconds=60)},
        "ip": {"concurrency": 4},
        "retry_after": {"max": timedelta(minutes=15)},
    }
    conn_site_prefs: ConnectionSitePreferencesSettings = {
        "purpose": collie_conn_site_prefs.get("purpose", "search"),
        "respect_use_signals": collie_conn_site_prefs.get("respect_use_signals", False),
        "respect_robots_txt": True,
        "max_crawl_delay": timedelta(seconds=30),
        "robots_txt_user_agent": "operator-bot",
    }
    run_site_prefs: RunSitePreferencesSettings = {
        "purpose": collie_run_site_prefs.get("purpose", "search"),
        "respect_use_signals": collie_run_site_prefs.get("respect_use_signals", False),
        "respect_robots_txt": True,
        "max_crawl_delay": timedelta(seconds=30),
        "robots_txt_user_agent": "operator-bot",
    }
    if "proxy" in collie_conn_politeness:
        conn_politeness["proxy"] = collie_conn_politeness["proxy"]
    if "proxy" in collie_run_politeness:
        run_politeness["proxy"] = collie_run_politeness["proxy"]
    del conn_politeness, run_politeness, conn_site_prefs, run_site_prefs
