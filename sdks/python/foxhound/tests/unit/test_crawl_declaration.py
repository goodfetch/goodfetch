"""A crawl's declaration (ADR 0093): Labels, handlers and Datasets, with no I/O.

The crawls here are declared at module scope, before any Connection exists,
as a program declares its own. A wiring mistake fails at once: a second
Label of one name or a second handler with `label_duplicate`, another
crawl's Label with `label_unknown`, and, before a Run starts, a Label
without a handler with `label_unhandled`. TypeScript's twin is the wiring
mistakes of `src/crawl.test.ts` of `@goodfetch/foxhound`.
"""

from __future__ import annotations

from datetime import timedelta
from typing import cast

import pytest
from goodfetch_foxhound.crawl import (
    AsyncCrawl,
    AsyncCrawlContext,
    Crawl,
    CrawlContext,
    check_startable,
    crawl_declaration,
    declared_label,
    handler_of,
    seed,
)
from goodfetch_foxhound.generated.errors import (
    LabelDuplicateError,
    LabelUnhandledError,
    LabelUnknownError,
    PayloadInvalidError,
)
from goodfetch_foxhound.generated.models import (
    DatasetDeclaration,
    LabelDeclaration,
    RequestSeed,
)
from pydantic import BaseModel, Field


class Shelf(BaseModel):
    """A listing page's Payload, numbered from 1."""

    number: int = Field(ge=1)


class Product(BaseModel):
    """One Record of the products Dataset."""

    id: str
    price: str


catalog = AsyncCrawl()
listing = catalog.label("listing", settings={"fetch": {"tier": "http"}})
shelf = catalog.label("shelf", payload=Shelf)
products = catalog.dataset("products", schema=Product, key=["id"])


@catalog.handle(listing)
async def read_listing(ctx: AsyncCrawlContext[None]) -> None:
    """Handles a listing page."""
    del ctx


@catalog.handle(shelf)
async def read_shelf(ctx: AsyncCrawlContext[Shelf]) -> None:
    """Handles a shelf page."""
    del ctx


pages = Crawl()
page = pages.label("page")


@pages.handle(page)
def read_page(ctx: CrawlContext[None]) -> None:
    """Handles a page."""
    del ctx


@pytest.mark.covers("story:11-1013")
def test_handle_returns_each_handler_unchanged_and_attaches_it() -> None:
    """The decorator hands the function back as written, attached to its Label."""
    assert (
        handler_of(catalog, "listing"),
        handler_of(catalog, "shelf"),
        handler_of(pages, "page"),
    ) == (read_listing, read_shelf, read_page)


@pytest.mark.covers("story:11-1013")
def test_declares_each_label_with_its_payload_schema_and_settings_on_the_wire() -> None:
    """A Payload's JSON Schema and a Label's settings go as `crawl.start` takes them."""
    crawl = AsyncCrawl()
    crawl.label("detail", settings={"timeout": {"hook": timedelta(seconds=2)}})
    crawl.label("book", payload={"type": "object", "required": ["isbn"]})
    crawl.label("shelf", payload=Shelf)

    labels, datasets = crawl_declaration(crawl)

    assert (labels, datasets) == (
        {
            "detail": LabelDeclaration(settings={"timeout": {"hook": 2000}}),
            "book": LabelDeclaration(payload={"type": "object", "required": ["isbn"]}),
            "shelf": LabelDeclaration(payload=Shelf.model_json_schema()),
        },
        [],
    )


def _issues(refused: PayloadInvalidError) -> tuple[object, list[object]]:
    """What a `payload_invalid` names: its Label and each issue's path."""
    issues = cast("list[dict[str, object]]", refused.details["issues"])
    return refused.details["label"], [issue["path"] for issue in issues]


@pytest.mark.covers("story:11-1018", "error:payload_invalid")
def test_refuses_a_seeds_payload_its_models_own_parse_refuses() -> None:
    """Pydantic's parse checks it in the program, before anything is sent."""
    with pytest.raises(PayloadInvalidError) as refused:
        seed("https://shop.test/shelf/0", shelf, Shelf.model_construct(number=0))

    assert (refused.value.code, _issues(refused.value)) == (
        "payload_invalid",
        ("shelf", ["/number"]),
    )


@pytest.mark.covers("story:11-1018", "error:payload_invalid")
def test_refuses_a_seeds_payload_that_is_not_json() -> None:
    """A hand-written schema has no parse in the program, but NaN is no JSON."""
    book = Crawl().label("book", payload={"type": "object"})

    with pytest.raises(PayloadInvalidError) as refused:
        seed("https://shop.test/book/1", book, {"isbn": float("nan")})

    assert _issues(refused.value) == ("book", ["/isbn"])


@pytest.mark.covers("story:11-1015")
def test_declares_each_dataset_with_its_schema_and_key() -> None:
    """A Dataset goes with its Records' JSON Schema and key, and without either."""
    _, datasets = crawl_declaration(catalog)
    crawl = Crawl()
    crawl.dataset("pages")
    _, bare = crawl_declaration(crawl)

    assert (datasets, bare) == (
        [
            DatasetDeclaration.model_validate(
                {
                    "name": "products",
                    "key": ["id"],
                    "schema": Product.model_json_schema(),
                }
            )
        ],
        [DatasetDeclaration(name="pages")],
    )


@pytest.mark.covers("error:label_duplicate")
def test_refuses_a_second_label_of_one_name_with_label_duplicate() -> None:
    """The first Label stays the crawl's, and nothing else is declared."""
    crawl = Crawl()
    crawl.label("page")

    with pytest.raises(LabelDuplicateError) as refused:
        crawl.label("page", payload=Shelf)

    assert (refused.value.code, refused.value.details) == (
        "label_duplicate",
        {"label": "page"},
    )


@pytest.mark.covers("error:label_duplicate")
def test_refuses_a_second_handler_for_one_label_with_label_duplicate() -> None:
    """The first handler stays attached."""
    with pytest.raises(LabelDuplicateError) as refused:
        pages.handle(page)(read_page)

    assert (refused.value.details, handler_of(pages, "page")) == (
        {"label": "page"},
        read_page,
    )


@pytest.mark.covers("error:label_unknown")
def test_refuses_a_handler_for_another_crawls_label_of_the_same_name() -> None:
    """`label_unknown` names the Label and those the crawl declares."""
    crawl = Crawl()
    crawl.label("page")

    with pytest.raises(LabelUnknownError) as refused:
        crawl.handle(page)(read_page)

    assert (refused.value.code, refused.value.details) == (
        "label_unknown",
        {"label": "page", "declared": ["page"]},
    )


@pytest.mark.covers("error:label_unknown")
def test_names_a_label_it_declares_and_refuses_another_crawls() -> None:
    """An enqueue's destination is the very Label the crawl's `label` returned."""
    other = AsyncCrawl().label("listing")

    with pytest.raises(LabelUnknownError) as refused:
        declared_label(catalog, other)

    assert (declared_label(catalog, listing), refused.value.details) == (
        "listing",
        {"label": "listing", "declared": ["listing", "shelf"]},
    )


@pytest.mark.covers("error:label_unhandled")
def test_refuses_to_start_a_crawl_with_a_label_left_without_a_handler() -> None:
    """`label_unhandled` names the Label without a handler."""
    crawl = AsyncCrawl()
    crawl.label("detail")

    with pytest.raises(LabelUnhandledError) as refused:
        check_startable(crawl, [])

    assert (refused.value.code, refused.value.details) == (
        "label_unhandled",
        {"label": "detail"},
    )


@pytest.mark.covers("error:label_unknown")
def test_refuses_to_start_a_crawl_from_a_seed_of_a_label_it_does_not_declare() -> None:
    """A seed names its Label by name, which the crawl must declare."""
    seeds = [RequestSeed(url="https://shop.test/a", label="detail")]

    with pytest.raises(LabelUnknownError) as refused:
        check_startable(pages, seeds)
    check_startable(pages, [RequestSeed(url="https://shop.test/a", label="page")])

    assert refused.value.details == {"label": "detail", "declared": ["page"]}
