"""A handler's `push` and its Records read back on an Attached engine (ADRs 0103, 0104).

On an Attached engine of Foxhound's executable of the same commit,
crawling the Proving ground's `catalog` family, each Record is checked in
the program before it is sent: one that is not JSON, or that its
Dataset's model would not parse back, is never staged, and
`record_invalid` names it by its position beside those the engine
refuses. Caught, the attempt keeps the others; escaped, the request fails
without a retry. Read back, each Record is its Dataset's type, a typed
read stops at the Record its schema refuses and resumes past it, and the
rejection table names what the engine refused. TypeScript's
`datasets.test.ts` of Foxhound's package is its twin.
"""

from __future__ import annotations

import contextlib
from typing import TYPE_CHECKING, Any

import anyio.to_thread
import pytest
from goodfetch_foxhound import AsyncCrawl, AsyncFoxhound, seed
from goodfetch_foxhound.generated.errors import RecordInvalidError
from goodfetch_test_support import Product as ToolProduct
from pydantic import BaseModel, ConfigDict, field_validator

if TYPE_CHECKING:
    from collections.abc import Mapping

    from goodfetch_foxhound import AsyncCrawlContext, AsyncCrawlRun, Dataset
    from goodfetch_test_support import ProvingGround

TASK = "catalog/1"
"""The Proving ground's task the crawl starts at, the catalog's first listing."""


class Product(BaseModel):
    """A product whose price has two decimals."""

    id: str
    name: str
    price: str

    @field_validator("price")
    @classmethod
    def _two_decimals(cls, price: str) -> str:
        whole, _, cents = price.partition(".")
        if not (whole.isdigit() and len(cents) == 2 and cents.isdigit()):
            message = "has no two decimals"
            raise ValueError(message)
        return price


Refused = list[tuple[int, list[str]]]
"""What a `record_invalid` names: each refused Record's position and issue paths."""


def _refused(error: RecordInvalidError) -> Refused:
    details = error.typed_details
    assert details is not None
    return [
        (record.position, [issue.path for issue in record.issues])
        for record in details.records
    ]


async def _values(run: AsyncCrawlRun, dataset: Dataset[Any]) -> list[object]:
    async with run.records(dataset) as read:
        return [record.value async for record in read]


@pytest.fixture
def product() -> ToolProduct:
    """The test runs Foxhound's executable."""
    return ToolProduct.FOXHOUND


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground's own seed for these tests, TypeScript's."""
    return 1486


@pytest.fixture
def with_browser() -> bool:
    """The engine serves each attempt a page, on the test's Browser cache."""
    return True


@pytest.mark.covers("story:11-1058", "story:11-1059", "error:record_invalid")
@pytest.mark.anyio
async def test_refuses_a_record_before_it_is_sent_and_fails_its_request_when_it_escapes(
    env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """Caught, the push keeps the Records that hold; escaped, it is not retried.

    A product built without validation would not parse back, and a note
    holds `NaN`: the program refuses both. The engine refuses the note
    without its key, named by its place in its push.
    """
    start = await anyio.to_thread.run_sync(proving_ground.start_url, TASK)
    crawl = AsyncCrawl()
    caught = crawl.label("caught")
    escaped = crawl.label("escaped")
    products = crawl.dataset("products", schema=Product, key=["id"])
    notes = crawl.dataset("notes", key=["id"])
    failures: list[Refused] = []
    escaped_attempts = 0

    @crawl.handle(caught)
    async def push_caught(ctx: AsyncCrawlContext[None]) -> None:
        free = Product.model_construct(id="P-2", name="Red kettle", price="free")
        try:
            await ctx.push(
                products,
                [
                    Product(id="P-1", name="Blue kettle", price="12.50"),
                    free,
                    Product(id="P-3", name="Green kettle", price="9.99"),
                ],
            )
        except RecordInvalidError as error:
            failures.append(_refused(error))
        # The program refuses the Record that is not JSON, and the engine
        # the one that lacks the key: each is named by its place in the push.
        try:
            await ctx.push(
                notes,
                [
                    {"id": "N-1", "text": 1},
                    {"id": "N-2", "text": float("nan")},
                    {"text": 3},
                ],
            )
        except RecordInvalidError as error:
            failures.append(_refused(error))

    @crawl.handle(escaped)
    async def push_escaped(ctx: AsyncCrawlContext[None]) -> None:
        nonlocal escaped_attempts
        escaped_attempts += 1
        lid = Product.model_construct(id="P-4", name="Kettle lid", price="1.5")
        await ctx.push(products, lid)

    foxhound = await AsyncFoxhound.connect(env=env)
    run = await foxhound.start_crawl(
        crawl,
        seeds=[seed(start, caught), seed(start, escaped, key="escaped")],
        settings={
            "fetch": {"tier": "http"},
            "crawl": {"handler": {"concurrency": 1}, "retry": {"handler_failed": True}},
        },
    )
    report = await run.finish()
    committed = (await _values(run, products), await _values(run, notes))
    await foxhound.close()

    assert failures == [[(1, ["/price"])], [(1, ["/text"]), (2, ["/id"])]]
    assert committed == (
        [
            Product(id="P-1", name="Blue kettle", price="12.50"),
            Product(id="P-3", name="Green kettle", price="9.99"),
        ],
        [{"id": "N-1", "text": 1}],
    )
    assert escaped_attempts == 1
    # `finish()` answers Collie's Run report until issue 1492 types
    # Foxhound's, so the failed request's Error code is read from its
    # Completeness reason.
    reasons = [
        (reason.code, reason.count, "(`record_invalid`: 1)" in reason.message)
        for reason in report.completeness_reasons or []
    ]
    assert reasons == [("failed_requests", 1, True)]


class StrictProduct(BaseModel):
    """A reader's own product, whose price must be a string."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    id: str
    name: str
    price: str


@pytest.mark.covers(
    "story:11-1060", "story:11-1061", "story:11-1070", "error:record_invalid"
)
@pytest.mark.anyio
async def test_reads_records_as_a_readers_type_past_after_and_the_rejection_table(
    env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """A typed read stops at the Record its schema refuses, and resumes past it.

    The engine refuses the Record without its key into the rejection table,
    which reads with its reason, issue, request and URL.
    """
    start = await anyio.to_thread.run_sync(proving_ground.start_url, TASK)
    shop = AsyncCrawl()
    listing = shop.label("listing")
    products = shop.dataset("products", key=["id"])

    @shop.handle(listing)
    async def push(ctx: AsyncCrawlContext[None]) -> None:
        with contextlib.suppress(RecordInvalidError):
            await ctx.push(
                products,
                [
                    {"id": "P-1", "name": "Blue kettle", "price": "12.50"},
                    {"id": "P-2", "name": "Red kettle", "price": 9.99},
                    {
                        "id": "P-3",
                        "name": "Green kettle",
                        "price": "3.00",
                        "colour": "green",
                    },
                    {"name": "Nameless kettle", "price": "1.00"},
                ],
            )

    foxhound = await AsyncFoxhound.connect(env=env)
    run = await foxhound.start_crawl(
        shop, seeds=[seed(start, listing)], settings={"fetch": {"tier": "http"}}
    )
    await run.finish()
    async with run.records(products) as read:
        numbers = {record.value.get("id"): record.number async for record in read}
    # A reader's own declaration of the Dataset, whose schema wants a string price.
    typed = AsyncCrawl().dataset("products", schema=StrictProduct, key=["id"])
    stopped: RecordInvalidError | None = None
    async with run.records(typed, after=numbers["P-1"]) as refusing:
        try:
            await anext(refusing)
        except RecordInvalidError as error:
            stopped = error
    async with run.records(typed, after=numbers["P-2"]) as read:
        resumed = [record.value async for record in read]
    async with run.rejected_records(products) as read:
        rejected = [entry async for entry in read]
    await foxhound.close()

    assert stopped is not None
    assert stopped.typed_details is not None
    assert (stopped.typed_details.dataset, stopped.typed_details.number) == (
        "products",
        numbers["P-2"],
    )
    assert [(type(value), value.model_dump()) for value in resumed] == [
        (StrictProduct, {"id": "P-3", "name": "Green kettle", "price": "3.00"})
    ]
    assert resumed[0].model_extra == {"colour": "green"}
    assert [
        (
            entry.number,
            entry.data,
            [issue.path for issue in entry.issues],
            entry.url,
        )
        for entry in rejected
    ] == [(1, {"name": "Nameless kettle", "price": "1.00"}, ["/id"], start)]
