"""Snippet for Time, waits, deadlines, cancellation and Limits (#467)."""

import asyncio
import logging
from datetime import timedelta
from decimal import Decimal

from goodfetch import AsyncGoodfetch, Count, DeadlineExceededError, Goodfetch, Limits, Money

log = logging.getLogger(__name__)


async def search_quotes() -> int:
    # Limits on this connection's work: a program may set or lower them, never raise them.
    async with AsyncGoodfetch.connect(limits=Limits(price=Money(amount=Decimal("2.00"), currency="EUR"))) as gf:
        # The whole Run may last two hours; past that it stops with a Checkpoint and can be resumed.
        async with await gf.collie.start(timeout=timedelta(hours=2), limits=Limits(model_tokens=200_000)) as run:
            page = await run.new_page()
            await page.goto("https://quotes.toscrape.com/search.aspx", timeout=timedelta(seconds=10))

            # Choosing an author posts the form back to search.aspx and reloads the tag list: the waiter is armed before the block runs (P02).
            async with page.expect_response("**/search.aspx") as tags:
                await page.locator("#author").select_option("Albert Einstein")
            await tags.value

            # The host's own deadline cancels the call and sends `$/cancel`; CancelledError propagates.
            async with asyncio.timeout(5):
                async with page.expect_response("**/filter.aspx"):
                    await page.get_by_role("button", name="Search").click()
            count = await page.locator(".results .quote").wait_for_count(Count(min=1))

            try:
                await page.wait_for_load_state("networkidle", timeout=None)  # explicitly no timeout
            except DeadlineExceededError as error:
                # Names what it waited for, what it saw and where the limit came from, e.g. `timeout.wait`.
                log.warning("%s (limit %s)", error, error.limit)
                raise

            await page.wait_for_timeout(1_000)  # type: ignore  # removed: the deprecation names the app-state waits
            return count


def search_quotes_blocking() -> None:
    with Goodfetch.connect() as gf, gf.collie.start(timeout=timedelta(minutes=30)) as run:
        page = run.new_page()
        with page.expect_response("**/search.aspx") as tags:
            page.locator("#author").select_option("Albert Einstein")
        log.info("tags reloaded: %s", tags.value.status)
