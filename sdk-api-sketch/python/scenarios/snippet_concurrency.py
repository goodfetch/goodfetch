"""Snippet for Concurrency model per language (#468)."""

import logging
import re
from concurrent.futures import ThreadPoolExecutor

from goodfetch import AsyncGoodfetch, AsyncRoute, Goodfetch, ListenerFailedError, Page, Route

log = logging.getLogger("bookshop")


async def first_books(limit: int) -> list[str]:
    """Runs under asyncio or Trio (AnyIO)."""
    async with AsyncGoodfetch.connect() as gf:
        async with await gf.collie.start() as run:
            page = await run.new_page()

            async def drop(route: AsyncRoute) -> None:
                await route.abort()

            # Side activity is observed with a listener, subscribed when called; `with` closes it (Q5, Q11).
            # A request that needs an answer runs `drop` as a task on this loop (Q8).
            with page.on("console", lambda message: log.debug(message.text)), page.route("**/*.png", drop):
                seen: set[str] = set()
                # A compiled pattern runs in the engine; a lookahead such as `(?=` would fail at the call.
                # The predicate runs here and may use the program's own variables (P22).
                async with page.expect_response(re.compile(r"/catalogue/page-\d+\.html$")), page.expect_event(
                    "response", predicate=lambda response: response.url not in seen
                ):
                    try:
                        await page.goto("https://books.toscrape.com/")
                        await page.locator("li.next a").click()
                    except ListenerFailedError as error:
                        # A listener that raised was removed; its exception surfaces on the page it watched.
                        log.error("listener for %s failed", error.details.event)
                        raise

            # A sequence read to its end; leaving the loop with `break` releases it, which unsubscribes (Q10).
            titles: list[str] = []
            async for record in run.records():
                title = record.get("title")
                if isinstance(title, str):
                    titles.append(title)
                if len(titles) >= limit:
                    break
            return titles


def in_a_notebook_cell() -> int:
    """A notebook cell runs on a thread whose event loop is already running: the sync client never touches it (Q1)."""
    with Goodfetch.connect() as gf, gf.collie.start() as run:
        page = run.new_page()

        def drop(route: Route) -> None:
            route.abort()  # runs on a thread of the connection's pool, never on its private loop thread

        with page.route("**/*.png", drop):
            page.goto("https://books.toscrape.com/")
            return page.locator("article.product_pod").wait_for_count(20)


def from_threads(urls: list[str]) -> None:
    """Every value of the sync API is safe to call from any thread at once (Q4)."""
    with Goodfetch.connect() as gf, gf.collie.start() as run, ThreadPoolExecutor(max_workers=4) as pool:

        def visit(url: str) -> None:
            page: Page = run.new_page()
            with page:
                page.goto(url)

        list(pool.map(visit, urls))
