"""Snippet for Redirect limits, truncated bodies and transport failures on the fetch tiers (#549, ADR 0118)."""

import logging
from datetime import timedelta

from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncCrawlContext,
    AsyncFoxhound,
    BlockedError,
    CrawlRequestFailed,
    ResponseTruncatedError,
    ResponseUnavailableError,
    TooManyRedirectsError,
    seed,
)

log = logging.getLogger(__name__)

# A link checker reads each redirect itself: a redirect status listed in `crawl.handle_statuses` is not followed, the
# one way not to follow, so the handler gets the 301 with its `Location`.
crawl = AsyncCrawl()
link = crawl.label("link", settings={"crawl": {"handle_statuses": [301, 302, 303, 307, 308, 404]}})


@crawl.handle(link)
async def on_link(ctx: AsyncCrawlContext[None]) -> None:
    # A cut body never reaches a handler, so there is no flag to forget to check (G1).
    _ = ctx.response.truncated  # type: ignore[attr-defined]  # pyright: ignore[reportAttributeAccessIssue]
    status = ctx.response.status
    if 300 <= status < 400:
        log.info("%s redirects to %s", ctx.response.url, ctx.response.headers.get("location"))


async def fetch_edges() -> None:
    """Every other crawl follows Fetch's redirects on every tier, at most `fetch.redirect.max`, and a refresh stub's
    target as one more hop; a loop, a cut body or a long `Retry-After` is a typed outcome."""
    async with (
        AsyncFoxhound.connect() as foxhound,
        await foxhound.start_crawl(
            crawl,
            seeds=[seed("https://example.com/", link)],
            settings={
                "fetch": {"redirect": {"max": 10}, "refresh": {"max_delay": timedelta(0)}},  # instant refreshes only
                "politeness": {"retry_after": {"max": timedelta(hours=1)}},  # wait out a site's maintenance hour
            },
        ) as run,
    ):
        async for outcome in run.outcomes(kinds=["failed"]):
            if not isinstance(outcome, CrawlRequestFailed):
                continue
            match outcome.error:
                case TooManyRedirectsError() as error:
                    log.warning("redirect loop: %s", error.details.chain)
                case ResponseTruncatedError() as error:
                    log.warning("cut body: %d bytes", error.details.received_bytes)
                case ResponseUnavailableError() as error if error.details.retry_after is not None:
                    # A failure, so `retry_failed()` re-admits it once the site is back.
                    log.info("site asked to come back at %s: %s", error.details.retry_after, error.fix)
                case _:
                    pass


async def collie_page() -> None:
    """A Collie page follows the browser's 20 redirects and never waits on a `Retry-After`: the program directs its own
    navigation, and reads the site's time from the error."""
    async with AsyncFoxhound.connect() as foxhound, await foxhound.start() as run, await run.new_page() as page:
        try:
            await page.goto("https://example.com/")
            api = await page.request.fetch("https://example.com/api/items", max_redirects=0)  # the 3xx itself
            log.info("api answered %d", api.status)
        except TooManyRedirectsError as error:
            log.warning("loop from %s", error.details.url)
        except ResponseTruncatedError:
            await page.reload()  # retryable: the program's own retry
        except BlockedError as error:
            if error.details.block_cause != "rate_limit":
                raise
            log.warning("rate limited until %s", error.details.retry_after)
