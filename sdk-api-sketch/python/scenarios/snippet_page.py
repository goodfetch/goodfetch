"""Snippet for Collie's page API (#472, ADR 0085)."""

import asyncio
import logging
from dataclasses import dataclass
from datetime import timedelta

from goodfetch import (
    AsyncDialog,
    AsyncGoodfetch,
    AsyncLocator,
    AsyncPage,
    AsyncResponse,
    AsyncRoute,
    Goodfetch,
    JsonValue,
    PageValueInvalidError,
    TargetNotUniqueError,
)

log = logging.getLogger("quotes")


@dataclass(frozen=True)
class Quote:
    text: str
    author: str
    tag: str


async def einstein_quotes() -> list[Quote]:
    """Golden scenario 01, "Fill a dependent form and read the results"."""
    async with AsyncGoodfetch.connect() as gf, await gf.collie.start() as run, await run.new_page() as page:
        await page.goto("https://quotes.toscrape.com/search.aspx")
        # Choosing an author posts the form back; the action waits for that navigation to commit, and the tag's
        # option is waited for by the guard like any other condition (K01).
        await page.get_by_label("Author").select_option("Albert Einstein")
        await page.get_by_label("Tag").select_option("inspirational")
        async with page.expect_response("**/filter.aspx"):  # armed before the click (ADR 0080)
            await page.get_by_role("button", name="Search").click()

        # An empty list is an answer, not an error (G1). Reads run concurrently on the page, each in one round trip.
        async def read(quote: AsyncLocator) -> Quote:
            return Quote(
                text=await quote.locator(".content").text_content(),
                author=await quote.locator(".author").text_content(),
                tag=await quote.locator(".tag").text_content(),
            )

        return list(await asyncio.gather(*(read(quote) for quote in await page.locator(".results .quote").all())))


async def click_next(page: AsyncPage) -> None:
    """Strictness: an ambiguous target is waited out like any other condition, then raises with every match."""
    try:
        await page.get_by_role("link", name="Next").click(timeout=timedelta(seconds=5))
    except TargetNotUniqueError as e:
        log.warning("%s", e)  # lists each match as `page.get_by_role("link", name="Next").nth(0)`, …
        await page.get_by_role("link", name="Next").first.click()  # the visible opt-out


async def read_scrolling_quotes(page: AsyncPage) -> list[JsonValue]:
    """Routes and dialogs answer requests; one that answers nothing falls through (golden scenario 06's browser part)."""

    async def media(route: AsyncRoute) -> None:
        if route.request.resource_type in ("image", "font"):
            await route.abort("blockedbyclient")
        # Settling nothing falls through to earlier routes, then to the network.

    async def confirm(dialog: AsyncDialog) -> None:
        if dialog.type == "confirm" and dialog.message.startswith("Are you sure"):
            await dialog.accept()
        # Otherwise the dialog gets its default: a confirm is dismissed (ADR 0014).

    pages: list[JsonValue] = []

    async def api(response: AsyncResponse) -> None:
        if "/api/quotes" in response.url and response.ok:
            pages.append(await response.json())

    with page.route("**/*", media), page.on("dialog", confirm), page.on("response", api):
        await page.goto("https://quotes.toscrape.com/scroll")
        await page.mouse.wheel(0, 10_000)  # input with no target: marked unguarded
        last = await page.wait_for_request_finished("**/api/quotes?page=*")
        final = await last.response()  # its body is kept: reading it never fails
        if final is not None:
            log.info("last page: %s", final.status)
    return pages


async def read_stats(page: AsyncPage) -> JsonValue:
    """Page functions: JavaScript source, one explicit JSON argument, JSON back."""
    long_tags = await page.evaluate(
        "({ min }) => [...document.querySelectorAll('.tag')].filter((tag) => tag.textContent.length > min).length",
        {"min": 3},
    )
    href = await page.locator("li.next > a").get_attribute("href")  # `None` when absent: an answer
    log.info("next page: %s", href)
    try:
        await page.evaluate("() => document.body")  # a DOM node is not JSON
    except PageValueInvalidError as e:
        log.error("%s", e)
    return long_tags


async def pay_in_frame(page: AsyncPage) -> None:
    """Frames are reached through locators, same-origin or not; popups and downloads are values."""
    await page.frame_locator("iframe[title='Checkout']").get_by_role("button", name="Pay").click()
    await page.frame_locator().get_by_text("Paid").wait_for()  # every frame searched, one frame resolved

    async with page.expect_event("popup") as info:
        await page.get_by_text("Open receipt").click()
    async with await info.value as receipt:  # a page of the same Session, closed here
        async with receipt.expect_event("download") as download:
            await receipt.get_by_role("button", name="PDF").click()
        await (await download.value).save_as("receipt.pdf")  # streamed to this machine

    async def no_thanks(_: object) -> None:
        await page.get_by_role("button", name="No thanks").click()

    with page.add_locator_handler(page.get_by_text("Sign up to our newsletter"), no_thanks):
        await page.get_by_role("link", name="Orders").click()


def heap_metrics() -> JsonValue:
    """The way down, in the sync API: raw CDP for one page, under ADR 0017's rules, Chromium only."""
    with Goodfetch.connect() as gf, gf.collie.start() as run, run.new_page() as page:
        with page.new_cdp_session() as cdp:
            cdp.send("Performance.enable")
            return cdp.send("Performance.getMetrics")
