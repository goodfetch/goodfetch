"""A program's crawl on Foxhound's executable (ADR 0093; issue 1468).

The catalog program of `foxhound_catalog.py` crawls the Proving ground's
catalog family on an Attached engine of Foxhound's executable of the same
commit, three ways: an `AsyncCrawl` on `AsyncFoxhound` under asyncio and
Trio, and a `Crawl` on the sync `Foxhound`, whose handler runs on the
Connection's thread pool. Each way's Records go to the Proving ground's
oracle, which judges them against the task, so every way gives the same
Records and the same finished report. A Payload that does not fit its
Label fails with `payload_invalid`, in the program or in the engine, and
a Run resumes under a changed Payload schema only once its pending
Payloads fit it (ADR 0082; issue 1470). TypeScript's `crawl.test.ts` of
Foxhound's package is its twin.
"""

from __future__ import annotations

import functools
from dataclasses import dataclass
from typing import TYPE_CHECKING, cast

import anyio
import anyio.to_thread
import pytest
from foxhound_catalog import AsyncCatalog, Catalog
from goodfetch_foxhound import AsyncCrawl, AsyncFoxhound, Foxhound, seed
from goodfetch_foxhound.generated.errors import (
    PayloadInvalidError,
    PayloadSchemaChangedError,
)
from goodfetch_test_support import Product as ToolProduct
from goodfetch_test_support import Way, judge_task, three_ways
from pydantic import BaseModel, Field

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Mapping

    from goodfetch_collie_client.generated.models import RunReport
    from goodfetch_collie_client.json_values import JSONObject
    from goodfetch_collie_client.schema import JSONSchema
    from goodfetch_foxhound import AsyncCrawlContext, AsyncCrawlRun
    from goodfetch_foxhound.generated.models import RequestSeed
    from goodfetch_foxhound.generated.settings import CrawlSettings
    from goodfetch_test_support import ClientCaseRecord, ProvingGround

TASK = "catalog/1"
"""The Proving ground's task the crawl starts at, the catalog's first listing."""

SETTINGS: CrawlSettings = {
    "fetch": {"tier": "http"},
    "crawl": {"handler": {"concurrency": 1}},
}
"""The Run's settings: plain HTTP fetches, and one handler at a time."""


@pytest.fixture
def product() -> ToolProduct:
    """The test runs Foxhound's executable."""
    return ToolProduct.FOXHOUND


@pytest.fixture
def proving_ground_seed() -> int:
    """The Proving ground's own seed for these tests, TypeScript's."""
    return 1467


@pytest.fixture
def with_browser() -> bool:
    """The engine serves each attempt a page, on the test's Browser cache."""
    return True


@dataclass(frozen=True)
class Crawled:
    """What a way's crawl gave: its Run's report and its Records' values."""

    report: RunReport
    records: list[object]


async def _crawl_async(env: Mapping[str, str], start: str) -> Crawled:
    """Crawls the catalog from `start` with `AsyncCrawl` on `AsyncFoxhound`."""
    catalog = AsyncCatalog()
    foxhound = await AsyncFoxhound.connect(env=env)
    run = await foxhound.start_crawl(
        catalog.crawl, seeds=[seed(start, catalog.listing)], settings=SETTINGS
    )
    report = await run.finish()
    async with run.records(catalog.products) as read:
        records: list[object] = [record.value async for record in read]
    await foxhound.close()
    return Crawled(report=report, records=records)


def _crawl_sync(env: Mapping[str, str], start: str) -> Crawled:
    """Crawls the catalog from `start` with `Crawl` on the sync `Foxhound`."""
    catalog = Catalog()
    with Foxhound.connect(env=env) as foxhound:
        run = foxhound.start_crawl(
            catalog.crawl, seeds=[seed(start, catalog.listing)], settings=SETTINGS
        )
        report = run.finish()
        with run.records(catalog.products) as read:
            records: list[object] = [record.value for record in read]
    return Crawled(report=report, records=records)


@pytest.mark.covers("story:11-1013", "story:11-1015")
@pytest.mark.client_case(id="crawl-catalog-program", mode="attached")
@three_ways
async def test_crawls_the_catalog_family_and_the_oracle_accepts_its_records(
    way: Way,
    env: Mapping[str, str],
    proving_ground: ProvingGround,
    client_case: ClientCaseRecord,
) -> None:
    """One Label, one handler, one keyed Dataset and one seed, each way."""
    start = await anyio.to_thread.run_sync(proving_ground.start_url, TASK)

    crawled = (
        await anyio.to_thread.run_sync(_crawl_sync, env, start)
        if way is Way.SYNC
        else await _crawl_async(env, start)
    )
    answer = await anyio.to_thread.run_sync(
        functools.partial(judge_task, proving_ground, TASK, crawled.records)
    )
    client_case.oracle_answer(answer)

    with client_case.check("finish() returns the finished Run's report"):
        assert (crawled.report.lifecycle, crawled.report.verdict) == (
            "finished",
            "verified",
        )
    with client_case.check("the oracle accepts the Records"):
        assert answer.outcome == "verified"


class Shelf(BaseModel):
    """A shelf's Payload, numbered from 1: Pydantic's parse checks it in the program."""

    number: int = Field(ge=1)


ISBN: JSONSchema = {
    "type": "object",
    "properties": {"isbn": {"type": "string"}},
    "required": ["isbn"],
}
"""A book's Payload schema, written by hand, which the engine alone checks."""

Refusal = tuple[object, list[object], object]
"""What a `payload_invalid` names: its Label, each issue's path, and a limit passed."""


def _refusal(error: PayloadInvalidError) -> Refusal:
    issues = cast("list[dict[str, object]]", error.details.get("issues", []))
    return (
        error.details.get("label"),
        [issue.get("path") for issue in issues],
        error.details.get("limit"),
    )


@pytest.mark.covers(
    "story:11-1018",
    "error:payload_invalid",
    "setting:crawl.payload.max_size",
    "operation:crawl.start",
    "operation:crawl_request.enqueue",
)
@pytest.mark.anyio
async def test_refuses_a_payload_its_schema_refuses_or_too_long_on_seeds_and_enqueue(
    env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """`payload_invalid` from the program's parse, the engine's schema and its size."""
    start = await anyio.to_thread.run_sync(proving_ground.start_url, TASK)
    crawl = AsyncCrawl()
    page = crawl.label("page")
    shelf = crawl.label("shelf", payload=Shelf)
    book = crawl.label("book", payload=ISBN)
    refused: list[Refusal] = []

    @crawl.handle(page)
    async def enqueue_refused(ctx: AsyncCrawlContext[None]) -> None:
        long: JSONObject = {"isbn": "9" * (65 * 1024)}
        enqueues: list[Callable[[], Awaitable[object]]] = [
            lambda: ctx.enqueue("/shelf", shelf, Shelf.model_construct(number=0)),
            lambda: ctx.enqueue("/isbn", book, {"isbn": 7}),
            lambda: ctx.enqueue("/long", book, long),
        ]
        for enqueue in enqueues:
            try:
                await enqueue()
            except PayloadInvalidError as error:
                refused.append(_refusal(error))

    @crawl.handle(shelf)
    async def read_shelf(ctx: AsyncCrawlContext[Shelf]) -> None:
        del ctx

    @crawl.handle(book)
    async def read_book(ctx: AsyncCrawlContext[JSONObject]) -> None:
        del ctx

    foxhound = await AsyncFoxhound.connect(env=env)
    # `seed` itself refuses a Payload not JSON and one the model refuses;
    # `start_crawl` one a hand-written schema refuses.
    seeds_refused: list[Refusal] = []
    refused_seeds: list[Callable[[], RequestSeed]] = [
        lambda: seed(start, shelf, Shelf.model_construct(number=0)),
        lambda: seed(start, book, {"isbn": float("nan")}),
        lambda: seed(start, book, {"isbn": 7}),
    ]
    for refused_seed in refused_seeds:
        try:
            await foxhound.start_crawl(crawl, seeds=[refused_seed()], settings=SETTINGS)
        except PayloadInvalidError as error:
            seeds_refused.append(_refusal(error))
    run = await foxhound.start_crawl(
        crawl, seeds=[seed(start, page)], settings=SETTINGS
    )
    report = await run.finish()
    await foxhound.close()

    assert seeds_refused == [
        ("shelf", ["/number"], None),
        ("book", ["/isbn"], None),
        ("book", ["/isbn"], None),
    ]
    assert refused == [
        ("shelf", ["/number"], None),
        ("book", ["/isbn"], None),
        ("book", [], "crawl.payload.max_size"),
    ]
    assert report.lifecycle == "finished"


@pytest.mark.covers(
    "story:11-1018", "error:payload_schema_changed", "operation:crawl.start"
)
@pytest.mark.anyio
async def test_resumes_a_run_whose_payload_schema_changed_once_its_pending_payloads_fit(
    env: Mapping[str, str], proving_ground: ProvingGround
) -> None:
    """A schema its pending Payloads fail refuses it: `payload_schema_changed`."""
    start = await anyio.to_thread.run_sync(proving_ground.start_url, TASK)
    titled: JSONSchema = {
        **ISBN,
        "properties": {"isbn": {"type": "string"}, "title": {"type": "string"}},
    }
    required: JSONSchema = {**titled, "required": ["isbn", "title"]}
    handled: list[object] = []

    async def start_books(
        foxhound: AsyncFoxhound,
        payload: JSONSchema,
        handler: Callable[[AsyncCrawlContext[JSONObject]], Awaitable[None]],
    ) -> AsyncCrawlRun:
        """Starts the Run `changed` of a crawl of books, their Payloads of `payload`."""
        crawl = AsyncCrawl()
        book = crawl.label("book", payload=payload)
        crawl.handle(book)(handler)
        return await foxhound.start_crawl(
            crawl,
            seeds=[
                seed(start, book, {"isbn": "0"}),
                seed(f"{start}?again", book, {"isbn": "1"}),
            ],
            settings=SETTINGS,
            run_id="changed",
        )

    async def never_answers(ctx: AsyncCrawlContext[JSONObject]) -> None:
        del ctx
        await anyio.sleep_forever()

    async def does_nothing(ctx: AsyncCrawlContext[JSONObject]) -> None:
        del ctx

    async def counts(ctx: AsyncCrawlContext[JSONObject]) -> None:
        handled.append(ctx.request.payload)

    first = await AsyncFoxhound.connect(env=env)
    # Its handler never answers, so the connection closes with both books pending.
    await start_books(first, ISBN, never_answers)
    await first.close()
    second = await AsyncFoxhound.connect(env=env)
    with pytest.raises(PayloadSchemaChangedError) as refused:
        await start_books(second, required, does_nothing)
    run = await start_books(second, titled, counts)
    report = await run.finish()
    await second.close()

    details = refused.value.details
    issues = cast("list[dict[str, object]]", details.get("issues", []))
    assert (
        details.get("label"),
        details.get("failing"),
        details.get("pending"),
        [issue.get("path") for issue in issues],
    ) == ("book", 2, 2, [""])
    assert report.lifecycle == "finished"
    assert len(handled) == 2
