"""A crawl of the Proving ground's catalog family, as a program writes it (#1468).

One Label for its listing pages, whose handler pushes each product the
page lists as one Record into the keyed Dataset `products` and enqueues the
page its Next link leads to, under its own Label: `AsyncCatalog`'s handler
is an `async def` function, `Catalog`'s a plain function. TypeScript's
`tests/programs/catalog.ts` of Foxhound's package is its twin.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from goodfetch_collie_client.generated.models import PageField
from goodfetch_foxhound import AsyncCrawl, Crawl

if TYPE_CHECKING:
    from goodfetch_collie_client.generated.models import PageRead
    from goodfetch_collie_client.json_values import JSONObject
    from goodfetch_foxhound import AsyncCrawlContext, CrawlContext

LISTING_FIELDS = {
    "ids": PageField.model_validate(
        {
            "list": {
                "locator": {"kind": "css", "selector": "li.product"},
                "attribute": "data-id",
            }
        }
    ),
    "names": PageField.model_validate(
        {"list": {"locator": {"kind": "css", "selector": "li.product a"}}}
    ),
    "prices": PageField.model_validate(
        {
            "list": {
                "locator": {"kind": "css", "selector": "li.product span.price"},
                "match": {"pattern": r"([0-9]+\.[0-9]{2})"},
            }
        }
    ),
    "next": PageField.model_validate(
        {"locator": {"kind": "css", "selector": "a[rel=next]"}, "attribute": "href"}
    ),
}
"""What a listing page's handler reads: each product's fields, and the Next link."""


class AsyncCatalog:
    """The catalog's crawl, its handler an `async def` function."""

    def __init__(self) -> None:
        """Declares the crawl, its Label, its Dataset and its handler."""
        self.crawl = AsyncCrawl()
        self.listing = self.crawl.label("listing")
        self.products = self.crawl.dataset("products", key=["id"])

        @self.crawl.handle(self.listing)
        async def read_listing(context: AsyncCrawlContext[None]) -> None:
            read = await context.page.read(LISTING_FIELDS)
            for record in _products(read):
                await context.push(self.products, record)
            next_page = read.values.get("next")
            if isinstance(next_page, str):
                await context.enqueue(next_page, self.listing)

        del read_listing


class Catalog:
    """The catalog's crawl, its handler a plain function."""

    def __init__(self) -> None:
        """Declares the crawl, its Label, its Dataset and its handler."""
        self.crawl = Crawl()
        self.listing = self.crawl.label("listing")
        self.products = self.crawl.dataset("products", key=["id"])

        @self.crawl.handle(self.listing)
        def read_listing(context: CrawlContext[None]) -> None:
            read = context.page.read(LISTING_FIELDS)
            for record in _products(read):
                context.push(self.products, record)
            next_page = read.values.get("next")
            if isinstance(next_page, str):
                context.enqueue(next_page, self.listing)

        del read_listing


def _products(read: PageRead) -> list[JSONObject]:
    """The products a listing page lists, each one Record, its price without its sign.

    Raises:
        ValueError: The page gave a field short, or not at all.
    """
    ids = _texts(read, "ids")
    names = _texts(read, "names")
    prices = _texts(read, "prices")
    if len(names) != len(ids) or len(prices) != len(ids):
        message = (
            f"the page lists {len(ids)} products"
            f" but {len(names)} names and {len(prices)} prices"
        )
        raise ValueError(message)
    return [
        {"id": id_, "name": name, "price": price}
        for id_, name, price in zip(ids, names, prices, strict=True)
    ]


def _texts(read: PageRead, field: str) -> list[str]:
    """The texts the list field `field` read.

    Raises:
        ValueError: The page did not give the field in full.
    """
    values: object = read.values.get(field)
    if not isinstance(values, list):
        message = f"the page gave no {field}"
        raise ValueError(message)
    texts: list[str] = []
    for value in values:  # pyright: ignore[reportUnknownVariableType]
        if not isinstance(value, str):
            message = f"the page gave no {field}"
            raise ValueError(message)
        texts.append(value)
    return texts
