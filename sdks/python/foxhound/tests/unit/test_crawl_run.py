"""An `AsyncCrawl` on the Scripted engine (ADR 0069).

The program declares its Labels, handlers and Dataset on an `AsyncCrawl`,
starts it on a Connection, and serves the engine's `crawl_request.handle`
requests beside the Connection's tasks, on asyncio and on Trio. The script
fails the test on a message it does not expect and on a step left unused.
TypeScript's `crawl.test.ts` is its twin.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, cast

import anyio
import pytest
from goodfetch_collie_client.generated.models import PageField
from goodfetch_foxhound import AsyncCrawl, AsyncCrawlContext, seed
from goodfetch_foxhound.crawl_run import handler_failed
from goodfetch_foxhound.testing import async_connect_over
from goodfetch_test_support import Finished, PlayedMessage, play_script

if TYPE_CHECKING:
    from goodfetch_collie_client.generated.models import PageRead
    from goodfetch_collie_client.json_values import JSONObject
    from goodfetch_foxhound.generated.models import DatasetRecord
    from goodfetch_test_support import PlayedScript

WAIT_SECONDS = 10.0
"""How long a test waits for its handler to start reading."""
WAIT_POLLS = 10_000
"""How many times, a millisecond apart, a test checks the Client's answers."""


def _field(field: object) -> PageField:
    return PageField.model_validate(field)


def _text(read: PageRead, field: str) -> str:
    value = read.values[field]
    if not isinstance(value, str):
        message = f"the page gave no {field}"
        raise TypeError(message)
    return value


def _answers(played: PlayedScript) -> int:
    """How many of the engine's requests the Client answered so far.

    An answer is the one message the Client sends with no method.
    """
    return sum(
        1
        for played_message in played.messages
        if isinstance(played_message, PlayedMessage)
        and played_message.from_ == "client"
        and "method" not in cast("dict[str, object]", played_message.message)
    )


async def _answered(played: PlayedScript, count: int) -> None:
    """Waits until the Client answered `count` of the engine's requests.

    The Scripted engine's player records each message as it reads it, and
    sets no event, so the wait polls it.
    """
    for _ in range(WAIT_POLLS):
        if _answers(played) >= count:
            return
        await anyio.sleep(0.001)
    message = f"the Client answered fewer than {count} requests"
    raise AssertionError(message)


@pytest.mark.covers("story:11-1013", "story:11-1014", "story:11-1015")
@pytest.mark.anyio
async def test_crawls_two_page_types_into_a_dataset() -> None:
    """Each handler call is answered, then `finish()` and a `records()` read."""
    crawl = AsyncCrawl()
    listing = crawl.label("listing")
    product = crawl.label("product")
    products = crawl.dataset("products", key=["id"])

    @crawl.handle(listing)
    async def read_listing(context: AsyncCrawlContext[None]) -> None:
        read = await context.page.read(
            {
                "products": _field(
                    {
                        "list": {
                            "locator": {"kind": "css", "selector": "li.product a"},
                            "attribute": "href",
                        }
                    }
                )
            }
        )
        for href in cast("list[str]", read.values["products"]):
            await context.enqueue(href, product)

    @crawl.handle(product)
    async def read_product(context: AsyncCrawlContext[None]) -> None:
        read = await context.page.read(
            {
                "id": _field({"locator": {"kind": "css", "selector": "span.id"}}),
                "name": _field({"locator": {"kind": "css", "selector": "h1.name"}}),
                "price": _field(
                    {
                        "locator": {"kind": "css", "selector": "span.price"},
                        "attribute": "data-price",
                    }
                ),
            }
        )
        record: JSONObject = {
            field: _text(read, field) for field in ("id", "name", "price")
        }
        await context.push(products, record)

    played = play_script("sdk-python/crawl/two-page-types.json")
    foxhound = await async_connect_over(played.transport)

    run = await foxhound.start_crawl(
        crawl, seeds=[seed("https://shop.test/catalog/1", listing)]
    )
    # The Scripted engine plays its steps in order, so the program finishes
    # once both handler calls were answered, as a crawl's Run finishes once
    # every work item is done.
    await _answered(played, 2)
    report = await run.finish()
    records: list[DatasetRecord] = []
    async with run.records(products) as read:
        records.extend([record async for record in read])
    await foxhound.close()

    assert (report.lifecycle, report.verdict) == ("finished", "verified")
    assert [record.value for record in records] == [
        {"id": "P-0001", "name": "Blue kettle", "price": "12.34"}
    ]
    assert played.done() == Finished()


@pytest.mark.covers("story:11-1013")
@pytest.mark.anyio
async def test_closing_the_run_cancels_each_attempt_and_waits_for_its_answer() -> None:
    """The read in flight is cancelled, the attempt answers `closed`, then the abort."""
    crawl = AsyncCrawl()
    page = crawl.label("page")
    reading = anyio.Event()
    stopped: list[str] = []

    @crawl.handle(page)
    async def read_title(context: AsyncCrawlContext[None]) -> None:
        reading.set()
        try:
            await context.page.read(
                {"title": _field({"locator": {"kind": "css", "selector": "h1"}})}
            )
        finally:
            stopped.append(context.request.id)

    played = play_script("sdk-python/crawl/closed-mid-attempt.json")
    foxhound = await async_connect_over(played.transport)

    run = await foxhound.start_crawl(
        crawl, seeds=[seed("https://shop.test/a", page)], run_id="nightly"
    )
    with anyio.fail_after(WAIT_SECONDS):
        await reading.wait()
    await anyio.wait_all_tasks_blocked()
    await run.close()
    await foxhound.close()

    assert stopped == ["1"]
    assert played.done() == Finished()


@pytest.mark.covers("story:11-1013")
@pytest.mark.anyio
async def test_answers_an_unknown_run_and_a_handlers_goodfetch_error() -> None:
    """A Run no crawl here runs is `run_not_found`; a handler's own error passes."""
    crawl = AsyncCrawl()
    page = crawl.label("page")

    @crawl.handle(page)
    async def read_page(context: AsyncCrawlContext[None]) -> None:
        await context.page.read(
            {"title": _field({"locator": {"kind": "css", "selector": "h1"}})}
        )

    played = play_script("sdk-python/crawl/handler-errors.json")
    foxhound = await async_connect_over(played.transport)

    await foxhound.start_crawl(
        crawl,
        seeds=[seed("https://shop.test/report.pdf", page)],
        run_id="nightly",
        tags=["reports"],
    )
    await _answered(played, 2)
    await foxhound.close()

    assert played.done() == Finished()


def test_names_a_handlers_own_exception_with_its_type_message_and_frames() -> None:
    """`handler_failed` carries what the handler raised, retryable (ADR 0109)."""

    def handler() -> None:
        message = "no price on the page"
        raise ValueError(message)

    try:
        handler()
    except ValueError as error:
        failed = handler_failed("product", error)
    else:
        pytest.fail("the handler raised nothing")

    stack = failed.details["exception_stack"]
    assert {
        "message": str(failed),
        "code": failed.code,
        "retryable": failed.retryable,
        "type": failed.details["exception_type"],
        "exception_message": failed.details["exception_message"],
        "names_the_handler": isinstance(stack, str) and "in handler" in stack,
    } == {
        "message": "the handler of the Label product failed with ValueError: "
        "no price on the page",
        "code": "handler_failed",
        "retryable": True,
        "type": "ValueError",
        "exception_message": "no price on the page",
        "names_the_handler": True,
    }
