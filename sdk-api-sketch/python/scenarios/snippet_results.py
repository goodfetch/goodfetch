"""Snippet for Results, outcomes and errors (#466): catch by group or code, read outcomes as values."""

import logging
from collections.abc import AsyncIterable, Awaitable, Callable
from dataclasses import dataclass

from goodfetch import (
    AiWorkResult,
    DeadlineExceededError,
    GoodfetchError,
    PageError,
    CrawlRequestBlocked,
    CrawlRequestFailed,
    CrawlRequestOutcome,
    CrawlRequestSkipped,
    Stopped,
    TargetNotUniqueError,
)

log = logging.getLogger(__name__)


@dataclass
class Book:
    title: str
    price: float


async def failures_raise(click_next: Callable[[], Awaitable[None]]) -> None:
    """`click_next` is a page call; the page API is #472's."""
    try:
        await click_next()
    except DeadlineExceededError as e:
        log.error("%s (limit: %s)", e, e.limit)  # the setting, its value and its layer
    except TargetNotUniqueError as e:
        log.error("%s: %s", e, e.details.matches)  # rendered in Python
    except PageError as e:
        log.error("%s at %s, observed %s", e, e.url, e.observed)  # any page problem, by group
    except GoodfetchError as e:
        if not e.retryable:
            raise
        await click_next()  # the SDK never retries on its own


async def outcomes_are_values(outcomes: AsyncIterable[CrawlRequestOutcome]) -> dict[str, int]:
    """How a program reads outcomes is #478's."""
    counts: dict[str, int] = {}
    async for outcome in outcomes:
        name = type(outcome).__name__
        counts[name] = counts.get(name, 0) + 1
        match outcome:
            case CrawlRequestFailed(error=error, attempts=attempts):
                log.error("%s after %d attempts", error.code, attempts)
            case CrawlRequestSkipped(reason=reason):
                log.info("skipped: %s", reason)
            case CrawlRequestBlocked(block_cause=cause):
                log.warning("blocked: %s", cause)
            case _:
                pass
    return counts


def ai_work(task: AiWorkResult[Book]) -> Book:
    """The Agent's API is ADR 0100's (`page.agent`)."""
    if isinstance(task.ended, Stopped):
        log.warning("stopped (%s), partial: %s", task.ended.reason, task.partial)
    claim = task.reported.claim if task.reported else None  # absent when the model claimed nothing
    log.info("%s; the model claimed %s", task.verdict, claim)  # a signal, never proof
    return task.require_verified()  # the output, or a NotVerifiedError carrying the Verdict
