"""A crawl's answer to `run.progress`, read into its generated type (ADR 0102).

Every Run's parts read with the crawl's own, instants as datetimes, and a
wait reason a newer engine sends kept as it came. Collie's `RunProgress`
keeps the crawl's parts untyped; the TypeScript twin is
`tests/crawl-run-progress.test.ts` of `@goodfetch/foxhound`.
"""

from __future__ import annotations

from datetime import UTC, datetime

from goodfetch_collie_client.caller import result_of
from goodfetch_foxhound.generated.models import CrawlRunProgress


def test_a_crawls_progress_reads_into_its_type_a_newer_wait_reason_kept() -> None:
    """Its waits, domains and concurrency typed; the newer reason as sent."""
    sent: dict[str, object] = {
        "run_id": "books",
        "position": 4,
        "at": "2026-01-05T09:00:00Z",
        "lifecycle": "running",
        "writer": "live",
        "epoch": 1,
        "pauses": [],
        "last": False,
        "waiting": [
            {
                "reason": "domain_delay",
                "count": 2,
                "domains": [
                    {"domain": "shop.test", "next_admitted_at": "2026-01-05T09:00:01Z"}
                ],
                "setting": {
                    "key": "politeness.domain.delay",
                    "value": 1000,
                    "layer": "run",
                },
            },
            {
                "reason": "a_reason_from_a_newer_engine",
                "count": 1,
                "domains": [{"domain": "a.test"}],
            },
        ],
        "idle": False,
        "concurrency": {"handlers": 3, "handler_concurrency": 8},
    }

    progress = result_of(CrawlRunProgress, sent)

    assert progress.waiting is not None
    assert progress.concurrency is not None
    assert (
        progress.at,
        progress.waiting[0].domains[0].next_admitted_at,
        [group.reason for group in progress.waiting],
        progress.idle,
        progress.concurrency.handler_concurrency,
    ) == (
        datetime(2026, 1, 5, 9, tzinfo=UTC),
        datetime(2026, 1, 5, 9, 0, 1, tzinfo=UTC),
        ["domain_delay", "a_reason_from_a_newer_engine"],
        False,
        8,
    )
