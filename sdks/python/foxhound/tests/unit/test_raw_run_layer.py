"""A deprecated key in the raw run layer `crawl.start` takes (ADR 0099).

`crawl.start` takes its `settings` as raw JSON, not as a typed scope, and
checks them against Foxhound's `DEPRECATED_SETTINGS` as a typed member does.
Collie's `test_run_time_markers.py` covers `run.resume`, in a process of its
own; no other Foxhound test sets `sample.echo_repeats`, so this one reads the
once-per-process warning in pytest's own process.
"""

from __future__ import annotations

import warnings

import pytest
from goodfetch_foxhound.generated.models import CrawlStartParams
from goodfetch_foxhound.generated.operations import FoxhoundOperations


class Refused(Exception):
    """What the engine answers every call with."""


class Engine:
    """An engine that refuses every call."""

    def call(self, operation: str, params: object) -> object:
        """Refuses `operation`, whatever its `params`."""
        raise Refused(operation, params)


@pytest.mark.covers("story:11-182")
def test_crawl_start_warns_once_per_process_of_a_deprecated_raw_key() -> None:
    """Starting two crawls whose raw `settings` set it warns once."""
    operations = FoxhoundOperations(Engine())

    with warnings.catch_warnings(record=True) as shown:
        warnings.simplefilter("always")
        for repeats in (3, 2):
            params = CrawlStartParams.model_validate(
                {
                    "seeds": [{"label": "start", "url": "https://example.com/"}],
                    "settings": {"sample": {"echo_repeats": repeats}},
                }
            )
            with pytest.raises(Refused):
                operations.crawl_start(params)

    assert [
        (warning.category, str(warning.message))
        for warning in shown
        if str(warning.message).startswith("The setting")
    ] == [
        (
            DeprecationWarning,
            "The setting `sample.echo_repeats` is deprecated. Since 0.1, removed "
            "no earlier than 1.0: A sample setting, which the real catalog replaces.",
        )
    ]
