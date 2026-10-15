"""Golden scenario 09, "Add an AI step to an existing Playwright program" (ADRs 0056, 0078, 0089, 0091)."""

import asyncio
import logging
from decimal import Decimal

from playwright.async_api import Page, async_playwright
from pydantic import BaseModel

from goodfetch_collie import AsyncCollie, GoodfetchError

log = logging.getLogger("books")


class Listed(BaseModel):
    title: str
    price: Decimal  # pounds


async def list_books(page: Page) -> None:
    """The added step. It borrows the page through the program's own Playwright connection (ADR 0056's relay), and
    leaving the block ends the borrow: the page stays open, where it was (ADR 0078)."""
    try:
        # Anthropic's key: env:ANTHROPIC_API_KEY, sent only to Anthropic.
        async with AsyncCollie.connect() as collie, await collie.start() as run:
            async with await run.borrow(page) as borrowed:
                result = await borrowed.locator("ol.row").extract(
                    "every book listed: title and price",
                    list[Listed],
                    model="anthropic/claude-sonnet-4-5",
                )
            print(result.output)
            await run.finish()
    except GoodfetchError as error:
        # Typed (`model_unavailable`, `extraction_incomplete`, `borrow_requires_cdp`, …), with its `fix`.
        log.warning(
            "The AI step failed (%s): %s; the program carries on", error.code, error.fix
        )


async def main() -> None:
    # The existing program, unchanged but for one call: it launches Chromium as Playwright does by default (over a
    # pipe, no port).
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch()
        page = await browser.new_page()
        await page.goto("https://books.toscrape.com/")
        await page.get_by_role("link", name="Travel").click()
        await list_books(page)
        await page.locator("article.product_pod h3 a").first.click()
        print(await page.locator(".product_main h1").text_content())
        await browser.close()


asyncio.run(main())
