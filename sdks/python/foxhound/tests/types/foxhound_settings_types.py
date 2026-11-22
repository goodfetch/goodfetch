"""A crawl's and a Label's settings (ADRs 0083 and 0093), checked by pyright and mypy.

Starting a crawl takes `CrawlSettings`, Collie's and Foxhound's Run keys,
and each of its Labels `LabelSettings`, the keys a Label sets alone:
nested `TypedDict` literals by the wire's `snake_case` segments, durations
as `timedelta`, `None` only where a setting can be cleared. A key outside
its scope, a misspelled or camelCase one and a wrong value are each
refused; mypy's `--strict` reports an ignore that no longer silences an
error. TypeScript's `settings.test-d.ts` of Foxhound's package is its twin.
"""

from datetime import timedelta
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from goodfetch_foxhound.generated.settings import (
        CrawlSettings,
        FoxhoundSettings,
        LabelSettings,
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
    del a, b, c, d, e, f, g, h
