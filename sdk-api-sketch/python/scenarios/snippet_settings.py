"""Snippet for Settings in code (#470), on golden scenarios 01, 10 and 12."""

import logging
from collections.abc import Mapping
from datetime import timedelta

from goodfetch import AsyncGoodfetch, ExplainedSetting, Goodfetch, SettingLockedError

log = logging.getLogger(__name__)


async def search_quotes() -> list[ExplainedSetting]:
    # One `settings` keyword per scope, inherited beneath it. `engine` is left at `auto`, so a `GOODFETCH_URL` in the
    # environment may fill it; a value written here would win.
    async with AsyncGoodfetch.connect(settings={"timeout": {"navigation": timedelta(seconds=60)}}) as gf:
        # A Session's identity is set where it opens, or above it, never on a page (ADR 0021).
        async with await gf.collie.start(
            settings={"browser": {"viewport": {"width": 1280, "height": 720}, "locale": "en-GB"}}
        ) as run:
            page = await run.new_page(settings={"timeout": {"action": timedelta(seconds=10)}})
            # A per-call argument applies to that call alone.
            await page.goto("https://quotes.toscrape.com/search.aspx", timeout=timedelta(seconds=20))

            # A wrong key fails pyright and mypy; built at runtime, the engine refuses it with `setting_unknown`.
            await run.new_page(settings={"timeout": {"navigate": timedelta(seconds=5)}})  # type: ignore[typeddict-unknown-key]  # pyright: ignore[reportArgumentType]
            # The Session's identity is not a page setting (`setting_not_applicable` at runtime).
            await run.new_page(settings={"browser": {"locale": "de-DE"}})  # type: ignore[typeddict-unknown-key]  # pyright: ignore[reportArgumentType]

            # What the page actually runs with, and which layer set each value (golden scenario 10 asserts on it).
            return await page.explain_settings(["timeout.action", "timeout.navigation", "browser.locale"])


def start_in_service(env: Mapping[str, str]) -> str:
    """A service reads no project file and passes its own environment map (golden scenario 12)."""
    with Goodfetch.connect(project=None, env=env) as gf:
        try:
            with gf.collie.start(settings={"timeout": {"run": None}}) as run:
                return run.id
        except SettingLockedError as error:
            # An Operator policy locked the Run's timeout: refused, never silently ignored; `limit` names the policy.
            log.error("%s", error, extra={"key": error.details.key, "locked_by": error.details.locked_by})
            raise
