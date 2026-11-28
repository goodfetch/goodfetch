"""A crawl's Records and rejected Records read back (ADRs 0103, 0104).

On the Scripted engine (ADR 0069), a crawl's Run, already finished, reads
its Dataset past `after`, each Record as its Dataset's own type with a
field its model does not declare kept, until one its model refuses, which
ends the read with `record_invalid` naming its number; and its rejection
table past `after`, page after page. TypeScript's `crawl.test.ts` of
`@goodfetch/foxhound` is its twin.
"""

from __future__ import annotations

import pytest
from goodfetch_foxhound import (
    AsyncCrawl,
    AsyncCrawlContext,
    Crawl,
    CrawlContext,
    seed,
)
from goodfetch_foxhound.generated.errors import RecordInvalidError
from goodfetch_foxhound.testing import async_connect_over, connect_over
from goodfetch_test_support import Finished, play_script
from pydantic import BaseModel, ConfigDict

PARSED = "sdk-python/crawl/records-parsed.json"
"""The Records numbered 2 and 3 read past 1, the latter's price no string."""
REJECTED = "sdk-python/crawl/rejected-records.json"
"""The rejection table read past 1, over two pages."""
SEED = "https://shop.test/a"
"""The crawl's one seed, a listing page."""
RUN_ID = "nightly"
"""The Run ID the scripts' engine answers as a Run that already finished."""
NOT_A_STRING = '23.45 is not of type "string"'
"""The schema's issue with the Record numbered 2's price."""
EXPECTED_REJECTED: list[dict[str, object]] = [
    {
        "number": 2,
        "data": {"id": "P-0002", "price": 23.45},
        "reason": f"the Dataset products refuses the Record: /price: {NOT_A_STRING}",
        "issues": [{"path": "/price", "message": NOT_A_STRING}],
        "hook": None,
        "request_id": "2",
        "url": "https://shop.test/products/P-0002",
    },
    {
        "number": 3,
        "data": {"id": "P-0003", "name": "Sample kettle", "price": "0.00"},
        "reason": "a free sample",
        "issues": [],
        "hook": "no-samples",
        "request_id": "3",
        "url": "https://shop.test/products/P-0003",
    },
]
"""The rejection table's entries past 1, as `rejected-records.json` pages them."""


class Product(BaseModel):
    """A reader's own product, whose price is a string, with no `colour`."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    id: str
    name: str
    price: str


def _async_crawl() -> AsyncCrawl:
    """The crawl the scripts start: two handled Labels and the Dataset `products`."""
    crawl = AsyncCrawl()

    async def handle(context: AsyncCrawlContext[None]) -> None:
        del context

    crawl.handle(crawl.label("listing"))(handle)
    crawl.handle(crawl.label("product"))(handle)
    crawl.dataset("products", key=["id"])
    return crawl


def _crawl() -> Crawl:
    """`_async_crawl`'s twin, with plain-function handlers."""
    crawl = Crawl()

    def handle(context: CrawlContext[None]) -> None:
        del context

    crawl.handle(crawl.label("listing"))(handle)
    crawl.handle(crawl.label("product"))(handle)
    crawl.dataset("products", key=["id"])
    return crawl


@pytest.mark.covers("story:11-1060", "story:11-1070", "error:record_invalid")
@pytest.mark.anyio
async def test_reads_records_past_after_as_the_datasets_type_until_one_it_refuses() -> (
    None
):
    """The model reads Record 2, keeping `colour`; Record 3 ends the read, named."""
    played = play_script(PARSED)
    foxhound = await async_connect_over(played.transport)
    run = await foxhound.start_crawl(
        _async_crawl(), seeds=[seed(SEED, "listing")], run_id=RUN_ID
    )
    # A reader's own declaration of the Dataset, whose model wants a string price.
    typed = AsyncCrawl().dataset("products", schema=Product, key=["id"])
    async with run.records(typed, after=1) as read:
        first = await anext(read)
        with pytest.raises(RecordInvalidError) as refused:
            await anext(read)
        with pytest.raises(StopAsyncIteration):
            await anext(read)
    await foxhound.close()

    assert (type(first.value), first.number, first.value.model_dump()) == (
        Product,
        2,
        {"id": "P-0002", "name": "Red kettle", "price": "23.45"},
    )
    assert first.value.model_extra == {"colour": "red"}
    details = refused.value.typed_details
    assert details is not None
    assert (details.dataset, details.number) == ("products", 3)
    assert [
        (record.position, [issue.path for issue in record.issues])
        for record in details.records
    ] == [(0, ["/price"])]
    assert played.done() == Finished()


@pytest.mark.covers("story:11-1060", "story:11-1070", "error:record_invalid")
def test_a_sync_run_reads_records_past_after_until_one_its_model_refuses() -> None:
    """The sync twin: the same Records, the same refusal of Record 3."""
    played = play_script(PARSED)
    with connect_over(played.transport) as foxhound:
        run = foxhound.start_crawl(
            _crawl(), seeds=[seed(SEED, "listing")], run_id=RUN_ID
        )
        typed = Crawl().dataset("products", schema=Product, key=["id"])
        with run.records(typed, after=1) as read:
            first = next(read)
            with pytest.raises(RecordInvalidError) as refused:
                next(read)

    assert (first.number, first.value.model_extra) == (2, {"colour": "red"})
    assert refused.value.typed_details is not None
    assert refused.value.typed_details.number == 3
    assert played.done() == Finished()


@pytest.mark.covers("story:11-1061")
@pytest.mark.anyio
async def test_reads_a_datasets_rejected_records_page_after_page_past_after() -> None:
    """Each entry reads as JSON with its reason, issues, Hook, request ID and URL."""
    played = play_script(REJECTED)
    foxhound = await async_connect_over(played.transport)
    run = await foxhound.start_crawl(
        _async_crawl(), seeds=[seed(SEED, "listing")], run_id=RUN_ID
    )
    products = AsyncCrawl().dataset("products", key=["id"])
    async with run.rejected_records(products, after=1) as read:
        rejected = [entry.model_dump() async for entry in read]
    await foxhound.close()

    assert rejected == EXPECTED_REJECTED
    assert played.done() == Finished()


@pytest.mark.covers("story:11-1061")
def test_a_sync_run_reads_a_datasets_rejected_records_page_after_page() -> None:
    """The sync twin: the same two pages, read in order."""
    played = play_script(REJECTED)
    with connect_over(played.transport) as foxhound:
        run = foxhound.start_crawl(
            _crawl(), seeds=[seed(SEED, "listing")], run_id=RUN_ID
        )
        products = Crawl().dataset("products", key=["id"])
        with run.rejected_records(products, after=1) as read:
            rejected = [entry.model_dump() for entry in read]

    assert rejected == EXPECTED_REJECTED
    assert played.done() == Finished()
