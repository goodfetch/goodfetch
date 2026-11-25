"""A crawl's Labels and handlers (ADRs 0082 and 0093), checked by pyright and mypy.

`label` has an overload for a Label without a Payload, one typed by a
Pydantic model and one by a JSON Schema written by hand, and a handler's
`ctx.request.payload` takes the Label's Payload type. An `AsyncCrawl`
takes `async def` handlers and a `Crawl` plain functions; a handler of
another Payload, a handler of the other kind, a misspelled or string
Label and a crawl started on the Connection of the other API are each
refused. A seed, an `enqueue` and an `enqueue_links` take a Payload of
their Label's type, and none for a Label without one. mypy's `--strict`
reports an ignore that no longer silences an error. TypeScript's
`payloads.test-d.ts` of Foxhound's package is its twin.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, assert_type

from goodfetch_foxhound import CrawlRun, seed, sitemap

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
    from goodfetch_foxhound.generated.models import MatchedLink
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


def pushes_only_records_of_a_datasets_type(
    ctx: CrawlContext[None], products: Dataset[Product]
) -> None:
    """A misspelled Dataset, a missing field and another Record are refused.

    A key is a name, which no type ties to a model's fields: one that names
    no field fails its declaration with `dataset_key_invalid`.
    """
    ctx.push(products, Product(id="p-1"))
    ctx.push(products, [Product(id="p-1"), Product(id="p-2")])
    ctx.push(productss, Product(id="p-1"))  # type: ignore[name-defined]
    ctx.push("products", Product(id="p-1"))  # type: ignore[arg-type]
    ctx.push(products, Product())  # type: ignore[call-arg]
    ctx.push(products, Shelf(number=1))  # type: ignore[misc]


async def pushes_only_records_of_a_datasets_type_in_async_code(
    ctx: AsyncCrawlContext[None], products: Dataset[Product]
) -> None:
    """The async context's `push` takes the same Records."""
    await ctx.push(products, Product(id="p-1"))
    await ctx.push(productss, Product(id="p-1"))  # type: ignore[name-defined]
    await ctx.push(products, Product())  # type: ignore[call-arg]
    await ctx.push(products, Shelf(number=1))  # type: ignore[misc]


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


def takes_a_payload_only_of_its_labels_type(crawl: Crawl) -> None:
    """A seed carries a Payload of its Label's type, and only for a Label with one.

    A JSON Schema's Payload is a JSON object, written in place.
    """
    listing = crawl.label("listing")
    shelf = crawl.label("shelf", payload=Shelf)
    book = crawl.label("book", payload={"type": "object"})
    seed("https://shop.test/", listing, priority=1)
    seed("https://shop.test/shelf/1", shelf, Shelf(number=1), priority=1)
    seed("https://shop.test/book/1", book, {"isbn": "9780000000001"})
    seed("https://shop.test/shelf/1", shelf, {"number": 1})  # type: ignore[arg-type]
    seed("https://shop.test/shelf/1", shelf)  # type: ignore[arg-type]
    seed("https://shop.test/", listing, Shelf(number=1))  # type: ignore[misc]
    seed("https://shop.test/book/1", book, "9780000000001")  # type: ignore[call-overload]


async def starts_from_a_sitemap_whose_rules_take_labels_without_a_payload(
    foxhound: AsyncFoxhound, crawl: AsyncCrawl
) -> None:
    """A sitemap entry carries no Payload, so a rule's Label takes none.

    A sitemap seed goes beside request seeds in one start.
    """
    listing = crawl.label("listing")
    shelf = crawl.label("shelf", payload=Shelf)
    await foxhound.start_crawl(
        crawl,
        seeds=[
            seed("https://shop.test/", listing),
            sitemap("https://shop.test/sitemap.xml", [("**/listing/*", listing)]),
        ],
    )
    sitemap("https://shop.test/sitemap.xml", [("**/shelf/*", shelf)])  # type: ignore[list-item]


async def enqueues_a_payload_only_of_its_labels_type(
    ctx: AsyncCrawlContext[None],
    listing: Label[None],
    shelf: Label[Shelf],
    book: Label[JSONObject],
) -> None:
    """A handler's `enqueue` and `enqueue_links` follow a seed's rules."""
    await ctx.enqueue("/shelf/2", shelf, Shelf(number=2))
    await ctx.enqueue("/book/2", book, {"isbn": "9780000000002"})
    await ctx.enqueue("/next", listing, priority=1)
    await ctx.enqueue_links(ctx.page.locator("a.shelf"), shelf, Shelf(number=3))
    await ctx.enqueue_links(ctx.page.locator("a.book"), book, {"from": "shelf"})
    await ctx.enqueue("/shelf/2", shelf, {"number": 2})  # type: ignore[arg-type]
    await ctx.enqueue("/next", listing, Shelf(number=2))  # type: ignore[misc]
    await ctx.enqueue_links(ctx.page.locator("a.shelf"), shelf)  # type: ignore[arg-type]


async def enqueues_links_with_a_payload_function(
    ctx: AsyncCrawlContext[None], shelf: Label[Shelf], book: Label[JSONObject]
) -> None:
    """`enqueue_links` takes a function of each link, plain or `async`, as Payload."""

    def shelf_of(link: MatchedLink) -> Shelf | None:
        return None if link.href is None else Shelf(number=len(link.text))

    async def book_of(link: MatchedLink) -> JSONObject | None:
        return {"from": link.text}

    def number_of(link: MatchedLink) -> int:
        return len(link.text)

    await ctx.enqueue_links(ctx.page.locator("a.shelf"), shelf, shelf_of, limit=2)
    await ctx.enqueue_links(ctx.page.locator("a.book"), book, book_of)
    await ctx.enqueue_links(ctx.page.locator("a.shelf"), shelf, number_of)  # type: ignore[arg-type]


def enqueues_links_with_a_payload_function_in_sync_code(
    ctx: CrawlContext[None], shelf: Label[Shelf]
) -> None:
    """The sync context's Payload function is a plain function."""

    def shelf_of(link: MatchedLink) -> Shelf | None:
        return None if link.href is None else Shelf(number=len(link.text))

    async def later(link: MatchedLink) -> Shelf | None:
        return shelf_of(link)

    ctx.enqueue_links(ctx.page.locator("a.shelf"), shelf, shelf_of)
    ctx.enqueue_links(ctx.page.locator("a.shelf"), shelf, later)  # type: ignore[arg-type]


def enqueues_a_payload_only_of_its_labels_type_in_sync_code(
    ctx: CrawlContext[None],
    listing: Label[None],
    shelf: Label[Shelf],
    book: Label[JSONObject],
) -> None:
    """The sync context's `enqueue` and `enqueue_links` follow the same rules."""
    ctx.enqueue("/shelf/2", shelf, Shelf(number=2))
    ctx.enqueue("/book/2", book, {"isbn": "9780000000002"})
    ctx.enqueue_links(ctx.page.locator("a.book"), book, {"from": "shelf"})
    ctx.enqueue("/shelf/2", shelf, {"number": 2})  # type: ignore[arg-type]
    ctx.enqueue("/next", listing, Shelf(number=2))  # type: ignore[misc]
    ctx.enqueue_links(ctx.page.locator("a.shelf"), shelf)  # type: ignore[arg-type]


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
