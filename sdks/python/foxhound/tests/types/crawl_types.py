"""A crawl's Labels and handlers (ADRs 0082 and 0093), checked by pyright and mypy.

`label` has an overload for a Label without a Payload, one typed by a
Pydantic model and one by a JSON Schema written by hand, and a handler's
`ctx.request.payload` takes the Label's Payload type. An `AsyncCrawl`
takes `async def` handlers and a `Crawl` plain functions; a handler of
another Payload, a handler of the other kind, a misspelled or string
Label and a crawl started on the Connection of the other API are each
refused. mypy's `--strict` reports an ignore that no longer
silences an error. TypeScript's `payloads.test-d.ts` of Foxhound's package
is its twin.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, assert_type

from goodfetch_foxhound import CrawlRun, seed

if TYPE_CHECKING:
    from goodfetch_collie_client.json_values import JSONObject
    from goodfetch_foxhound import (
        AsyncCrawl,
        AsyncCrawlContext,
        AsyncFoxhound,
        Crawl,
        CrawlContext,
        Dataset,
        Foxhound,
        Label,
    )
    from pydantic import BaseModel

    class Shelf(BaseModel):
        """A shelf page's Payload."""

        number: int

    class Product(BaseModel):
        """A product Record."""

        id: str


def types_each_payload_by_its_label(crawl: AsyncCrawl) -> None:
    """No Payload is `None`, a model its own type, a JSON Schema a JSON object."""
    listing = crawl.label("listing")
    shelf = crawl.label("shelf", payload=Shelf)
    book = crawl.label("book", payload={"type": "object"})
    assert_type(listing, "Label[None]")
    assert_type(shelf, "Label[Shelf]")
    assert_type(book, "Label[JSONObject]")

    @crawl.handle(shelf)
    async def read_shelf(ctx: AsyncCrawlContext[Shelf]) -> None:
        assert_type(ctx.request.payload, "Shelf")

    @crawl.handle(book)
    async def read_book(ctx: AsyncCrawlContext[JSONObject]) -> None:
        assert_type(ctx.request.payload, "JSONObject")

    @crawl.handle(listing)
    async def read_listing(ctx: AsyncCrawlContext[None]) -> None:
        assert_type(ctx.request.payload, None)

    del read_shelf, read_book, read_listing


def types_each_record_by_its_dataset(crawl: Crawl) -> None:
    """A model's Dataset holds the model, any other a JSON object."""
    assert_type(crawl.dataset("products", schema=Product), "Dataset[Product]")
    assert_type(crawl.dataset("pages", key=["url"]), "Dataset[JSONObject]")
    assert_type(crawl.dataset("raw", schema={"type": "object"}), "Dataset[JSONObject]")


def types_a_sync_crawls_handlers(crawl: Crawl) -> None:
    """A plain function takes the sync context, its Payload typed alike."""
    shelf = crawl.label("shelf", payload=Shelf)

    @crawl.handle(shelf)
    def read_shelf(ctx: CrawlContext[Shelf]) -> None:
        assert_type(ctx.request.payload, "Shelf")

    del read_shelf


def refuses_a_handler_of_another_payload(crawl: AsyncCrawl) -> None:
    """A handler typed for another Label's Payload is not this Label's."""
    listing = crawl.label("listing")

    @crawl.handle(listing)  # type: ignore[arg-type]
    async def read_shelf(ctx: AsyncCrawlContext[Shelf]) -> None:
        del ctx

    del read_shelf


def refuses_a_handler_of_the_other_kind(
    async_crawl: AsyncCrawl, crawl: Crawl, label: Label[None]
) -> None:
    """An `AsyncCrawl` takes no plain function, a `Crawl` no `async def`."""

    @async_crawl.handle(label)  # type: ignore[arg-type]
    def plain(ctx: AsyncCrawlContext[None]) -> None:
        del ctx

    @crawl.handle(label)  # type: ignore[arg-type]
    async def coroutine(ctx: CrawlContext[None]) -> None:
        del ctx

    del plain, coroutine


def refuses_an_async_crawl_on_the_sync_api(
    foxhound: Foxhound, async_crawl: AsyncCrawl, crawl: Crawl, label: Label[None]
) -> None:
    """`Foxhound` starts a `Crawl`, whose Run is a `CrawlRun`, and no `AsyncCrawl`."""
    seeds = [seed("https://shop.test/", label)]
    assert_type(foxhound.start_crawl(crawl, seeds=seeds), CrawlRun)
    foxhound.start_crawl(async_crawl, seeds=seeds)  # type: ignore[arg-type]


async def refuses_a_sync_crawl_on_the_async_api(
    foxhound: AsyncFoxhound, crawl: Crawl, label: Label[None]
) -> None:
    """`AsyncFoxhound` starts an `AsyncCrawl` alone."""
    await foxhound.start_crawl(crawl, seeds=[seed("https://shop.test/", label)])  # type: ignore[arg-type]


def refuses_a_misspelled_or_string_label(crawl: AsyncCrawl) -> None:
    """A Label is a handle, never its name: a misspelling is a type error."""
    listing = crawl.label("listing")

    @crawl.handle(listingg)  # type: ignore[name-defined]
    async def misspelled(ctx: AsyncCrawlContext[None]) -> None:
        del ctx

    @crawl.handle("listing")  # type: ignore[arg-type]
    async def named(ctx: AsyncCrawlContext[None]) -> None:
        del ctx

    del listing, misspelled, named


def refuses_a_payload_that_is_no_model_or_schema(crawl: Crawl) -> None:
    """A Payload type is a Pydantic model or a JSON Schema written by hand."""
    crawl.label("isbn", payload=str)  # type: ignore[type-var]


def refuses_a_label_option_it_does_not_take(crawl: Crawl) -> None:
    """A Label's settings are those a Label sets alone."""
    crawl.label("page", settings={"politeness": {"domain": {"concurrency": 2}}})  # type: ignore[call-overload]


def keeps_two_labels_payloads_apart(
    shelf: Label[Shelf], products: Dataset[Product]
) -> None:
    """A `Label[Shelf]` is no `Label[None]`, a `Dataset[Product]` no other Dataset."""
    no_payload: Label[None] = shelf  # type: ignore[assignment]
    raw: Dataset[JSONObject] = products  # type: ignore[assignment]
    del no_payload, raw
