"""Snippet for Crawl politeness defaults (#480), on golden scenario 05."""

import logging
from collections import Counter
from collections.abc import AsyncIterable
from datetime import timedelta

from goodfetch import AsyncGoodfetch, ExplainedSetting, RequestOutcome, RequestSkipped

log = logging.getLogger(__name__)


async def polite_by_default() -> list[ExplainedSetting]:
    """With nothing set, a crawl obeys robots.txt and keeps one request in flight per domain, one second between starts:
    scenario 05 sets nothing and is polite to books.toscrape.com (G5)."""
    async with AsyncGoodfetch.connect() as gf:
        # Each comes from the `default` layer unless configuration or an Operator policy set it.
        return await gf.explain_settings(
            [
                "politeness.domain.concurrency",
                "politeness.domain.delay",
                "site_preferences.respect_robots_txt",
                "site_preferences.max_crawl_delay",
            ]
        )


async def own_staging_site() -> None:
    """The program's own staging site, which it may press harder: an explicit value, recorded in the Audit log and
    listed in the run's result, and logged once as a warning through `logging`."""
    async with AsyncGoodfetch.connect(
        settings={
            "politeness": {"domain": {"concurrency": 8, "delay": timedelta(0)}},
            "site_preferences": {"respect_robots_txt": False},
        }
    ) as gf:
        # A misspelled key fails pyright and mypy; built at runtime, `setting_unknown` names `respect_robots_txt`.
        await AsyncGoodfetch.connect(settings={"site_preferences": {"respect_robots": False}})  # type: ignore[typeddict-unknown-key]  # pyright: ignore[reportArgumentType]
        # A Collie Run's program directs its own navigation: no per-domain limit (`setting_not_applicable` at runtime).
        await gf.collie.start(settings={"politeness": {"domain": {"concurrency": 2}}})  # type: ignore[typeddict-unknown-key]  # pyright: ignore[reportArgumentType]
        # Per-proxy limits apply to every run, a Collie Run included.
        async with await gf.collie.start(settings={"politeness": {"proxy": {"rate": 5.0}}}) as run:
            log.info("started %s", run.id)


async def as_own_bot() -> list[ExplainedSetting]:
    """An operator crawling under its own bot name, which its user agent carries: a site's robots.txt group for
    `AcmeBot` binds it too, merged with Foxhound's. Setting the token is recorded."""
    async with AsyncGoodfetch.connect(settings={"site_preferences": {"robots_txt_user_agent": "AcmeBot"}}) as gf:
        return await gf.explain_settings(["site_preferences.robots_txt_user_agent"])


async def why_skipped(outcomes: AsyncIterable[RequestOutcome]) -> Counter[str]:
    """A page the site asked crawlers to leave is skipped, a value with its reason, never a failure (G1). How a program
    reads outcomes is #478's."""
    skipped: Counter[str] = Counter()
    async for outcome in outcomes:
        if not isinstance(outcome, RequestSkipped):
            continue
        skipped[outcome.reason] += 1
        match outcome.reason:
            case "robots_txt":
                log.info("disallowed by the host's robots.txt")
            case "crawl_delay":
                log.warning("the host's Crawl-delay is above site_preferences.max_crawl_delay")
            case "use_signals":
                log.info("an AI-use signal refuses this run's Purpose")
            case reason:
                log.info("skipped: %s", reason)
    return skipped
