"""An `AsyncCrawl` and a sync `Crawl` on the Scripted engine (ADR 0069).

The program declares its Labels, handlers and Dataset on an `AsyncCrawl`,
starts it on a Connection, and serves the engine's `crawl_request.handle`
requests beside the Connection's tasks, on asyncio and on Trio; a sync
`Crawl`'s handlers run on the Connection's thread pool. The script fails
the test on a message it does not expect and on a step left unused.
TypeScript's `crawl.test.ts` is its twin.
"""

from __future__ import annotations

import threading
import time
from typing import TYPE_CHECKING, cast

import anyio
import pytest
from goodfetch_collie_client.generated.errors import ClosedError
from goodfetch_collie_client.generated.models import PageField
from goodfetch_collie_client.portal import THREAD_NAME
from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncCrawlContext,
    Crawl,
    CrawlContext,
    seed,
)
from goodfetch_foxhound.crawl_run import handler_failed
from goodfetch_foxhound.generated.errors import PayloadInvalidError
from goodfetch_foxhound.testing import async_connect_over, connect_over
from goodfetch_test_support import Finished, PlayedMessage, play_script

if TYPE_CHECKING:
    from collections.abc import Callable

    from goodfetch_collie_client.generated.models import PageRead
    from goodfetch_collie_client.json_values import JSONObject
    from goodfetch_collie_client.schema import JSONSchema
    from goodfetch_foxhound.generated.models import DatasetRecord, EnqueuedLinks
    from goodfetch_test_support import PlayedScript

WAIT_SECONDS = 10.0
"""How long a test waits for its handler to start reading."""
WAIT_POLLS = 10_000
"""How many times, a millisecond apart, a test checks the Client's answers."""


def _field(field: object) -> PageField:
    return PageField.model_validate(field)


def _css(selector: str, attribute: str | None = None) -> PageField:
    """The field reading the first element `selector` matches, or its `attribute`."""
    field: dict[str, object] = {"locator": {"kind": "css", "selector": selector}}
    if attribute is not None:
        field["attribute"] = attribute
    return _field(field)


LISTING_FIELDS = {
    "products": _field(
        {
            "list": {
                "locator": {"kind": "css", "selector": "li.product a"},
                "attribute": "href",
            }
        }
    )
}
"""What the listing's handler reads: the link of each product."""
PRODUCT_FIELDS = {
    "id": _css("span.id"),
    "name": _css("h1.name"),
    "price": _css("span.price", "data-price"),
}
"""What the product's handler reads, the fields of its Record."""
EXPECTED_RECORDS = [{"id": "P-0001", "name": "Blue kettle", "price": "12.34"}]
"""The Records `two-page-types.json`'s crawl commits."""


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


def _waited(played: PlayedScript, done: Callable[[PlayedScript], bool]) -> None:
    """Waits, blocking the thread, until `done` holds of what the script played."""
    for _ in range(WAIT_POLLS):
        if done(played):
            return
        time.sleep(0.001)
    message = "the script never played what the test waits for"
    raise AssertionError(message)


def _sent(played: PlayedScript, method: str) -> bool:
    """Whether the Client sent a message of `method` so far."""
    return any(
        isinstance(played_message, PlayedMessage)
        and played_message.from_ == "client"
        and cast("dict[str, object]", played_message.message).get("method") == method
        for played_message in played.messages
    )


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
        read = await context.page.read(LISTING_FIELDS)
        for href in cast("list[str]", read.values["products"]):
            await context.enqueue(href, product)

    @crawl.handle(product)
    async def read_product(context: AsyncCrawlContext[None]) -> None:
        read = await context.page.read(PRODUCT_FIELDS)
        record: JSONObject = {field: _text(read, field) for field in PRODUCT_FIELDS}
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
    assert [record.value for record in records] == EXPECTED_RECORDS
    assert played.done() == Finished()


@pytest.mark.covers("story:11-1013", "story:11-1015", "story:11-1025", "story:11-1104")
def test_runs_a_sync_crawls_handlers_on_the_connections_thread_pool() -> None:
    """The same crawl with plain-function handlers, each on a worker thread."""
    crawl = Crawl()
    listing = crawl.label("listing")
    product = crawl.label("product")
    products = crawl.dataset("products", key=["id"])
    threads: set[str] = set()

    @crawl.handle(listing)
    def read_listing(context: CrawlContext[None]) -> None:
        threads.add(threading.current_thread().name)
        read = context.page.read(LISTING_FIELDS)
        for href in cast("list[str]", read.values["products"]):
            context.enqueue(href, product)

    @crawl.handle(product)
    def read_product(context: CrawlContext[None]) -> None:
        threads.add(threading.current_thread().name)
        read = context.page.read(PRODUCT_FIELDS)
        record: JSONObject = {field: _text(read, field) for field in PRODUCT_FIELDS}
        context.push(products, record)

    played = play_script("sdk-python/crawl/two-page-types.json")
    with connect_over(played.transport) as foxhound:
        run = foxhound.start_crawl(
            crawl, seeds=[seed("https://shop.test/catalog/1", listing)]
        )
        _waited(played, lambda played: _answers(played) >= 2)
        report = run.finish()
        with run.records(products) as read:
            records = list(read)

    assert (report.lifecycle, report.verdict) == ("finished", "verified")
    assert [record.value for record in records] == EXPECTED_RECORDS
    # Neither the program's thread nor the loop's, which reads every answer.
    assert threads
    assert threads.isdisjoint({threading.current_thread().name, THREAD_NAME})
    assert played.done() == Finished()


@pytest.mark.covers("story:11-1013", "story:11-1104")
def test_closing_a_sync_run_cancels_its_handlers_call_and_waits_for_it() -> None:
    """The handler's read in flight is cancelled and raises `closed`, then the abort."""
    crawl = Crawl()
    page = crawl.label("page")
    raised: list[str] = []

    @crawl.handle(page)
    def read_title(context: CrawlContext[None]) -> None:
        try:
            context.page.read({"title": _css("h1")})
        except ClosedError as error:
            raised.append(str(error))
            raise

    played = play_script("sdk-python/crawl/closed-mid-attempt.json")
    with connect_over(played.transport) as foxhound:
        run = foxhound.start_crawl(
            crawl, seeds=[seed("https://shop.test/a", page)], run_id="nightly"
        )
        _waited(played, lambda played: _sent(played, "page.read"))
        run.close()

    assert raised == ["the Run nightly is closed, which stopped the handler's attempt"]
    assert played.done() == Finished()


FROM_SCHEMA: JSONSchema = {
    "type": "object",
    "properties": {"from": {"type": "string"}},
    "required": ["from"],
}
"""The Payload schema of `enqueue-links.json`'s Label, written by hand."""
FROM_START: JSONObject = {"from": "start"}
"""The seed's Payload."""
FROM_LISTING: JSONObject = {"from": "listing"}
"""The one Payload of every link the handler enqueues."""
ENQUEUED_LINKS = [
    {"kind": "added", "id": "2", "url": "https://shop.test/b"},
    {"kind": "not_added", "reason": "duplicate", "url": "https://shop.test/a"},
    {"kind": "no_link"},
]
"""What `enqueue-links.json`'s engine answers of each element the locator matches."""


@pytest.mark.covers("operation:crawl_request.enqueue_links", "story:11-1018")
@pytest.mark.anyio
async def test_enqueues_the_links_a_locator_matches_with_one_payload() -> None:
    """After a read and a push, one call enqueues the links: four exchanges a page."""
    crawl = AsyncCrawl()
    page = crawl.label("page", payload=FROM_SCHEMA)
    titles = crawl.dataset("pages")
    enqueued: list[EnqueuedLinks] = []

    @crawl.handle(page)
    async def read_page(context: AsyncCrawlContext[JSONObject]) -> None:
        read = await context.page.read({"title": _css("h1")})
        record: JSONObject = {"title": _text(read, "title")}
        await context.push(titles, record)
        links = await context.enqueue_links(
            context.page.locator("ul.pages a"),
            page,
            FROM_LISTING,
            exclude=["*/logout"],
            limit=2,
        )
        enqueued.append(links)

    played = play_script("sdk-python/crawl/enqueue-links.json")
    foxhound = await async_connect_over(played.transport)

    await foxhound.start_crawl(
        crawl,
        seeds=[seed("https://shop.test/a", page, FROM_START)],
        run_id="nightly",
    )
    await _answered(played, 1)
    await foxhound.close()

    assert [
        link.model_dump(mode="json", exclude_none=True)
        for answered in enqueued
        for link in answered.links
    ] == ENQUEUED_LINKS
    assert played.done() == Finished()


@pytest.mark.covers("story:11-1018", "error:payload_invalid")
@pytest.mark.anyio
async def test_refuses_a_payload_that_is_not_json_at_enqueue_sending_nothing() -> None:
    """`enqueue` and `enqueue_links` fail with `payload_invalid`; nothing is sent."""
    crawl = AsyncCrawl()
    page = crawl.label("page", payload=FROM_SCHEMA)
    titles = crawl.dataset("pages")
    refused: list[tuple[object, list[object]]] = []

    @crawl.handle(page)
    async def read_page(context: AsyncCrawlContext[JSONObject]) -> None:
        not_json: JSONObject = {"from": float("nan")}
        links = context.page.locator("ul.pages a")
        for enqueued in (
            lambda: context.enqueue("/nan", page, not_json),
            lambda: context.enqueue_links(links, page, not_json),
        ):
            try:
                await enqueued()
            except PayloadInvalidError as error:
                issues = cast("list[dict[str, object]]", error.details["issues"])
                refused.append((error.details["label"], [i["path"] for i in issues]))
        read = await context.page.read({"title": _css("h1")})
        record: JSONObject = {"title": _text(read, "title")}
        await context.push(titles, record)
        await context.enqueue_links(
            links, page, FROM_LISTING, exclude=["*/logout"], limit=2
        )

    played = play_script("sdk-python/crawl/enqueue-links.json")
    foxhound = await async_connect_over(played.transport)

    await foxhound.start_crawl(
        crawl,
        seeds=[seed("https://shop.test/a", page, FROM_START)],
        run_id="nightly",
    )
    await _answered(played, 1)
    await foxhound.close()

    assert refused == [("page", ["/from"]), ("page", ["/from"])]
    assert played.done() == Finished()


@pytest.mark.covers("operation:crawl_request.enqueue_links", "story:11-1018")
def test_enqueues_the_links_a_locator_matches_from_a_sync_handler() -> None:
    """The same handler as a plain function, on the Connection's thread pool."""
    crawl = Crawl()
    page = crawl.label("page", payload=FROM_SCHEMA)
    titles = crawl.dataset("pages")
    enqueued: list[EnqueuedLinks] = []

    @crawl.handle(page)
    def read_page(context: CrawlContext[JSONObject]) -> None:
        read = context.page.read({"title": _css("h1")})
        record: JSONObject = {"title": _text(read, "title")}
        context.push(titles, record)
        links = context.enqueue_links(
            context.page.locator("ul.pages a"),
            page,
            FROM_LISTING,
            exclude=["*/logout"],
            limit=2,
        )
        enqueued.append(links)

    played = play_script("sdk-python/crawl/enqueue-links.json")
    with connect_over(played.transport) as foxhound:
        foxhound.start_crawl(
            crawl,
            seeds=[seed("https://shop.test/a", page, FROM_START)],
            run_id="nightly",
        )
        _waited(played, lambda played: _answers(played) >= 1)

    assert [
        link.model_dump(mode="json", exclude_none=True)
        for answered in enqueued
        for link in answered.links
    ] == ENQUEUED_LINKS
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
