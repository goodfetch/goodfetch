"""Snippet for Typed data: schemas, Records and Payloads (#469), on golden scenarios 02 and 05."""

from collections.abc import Sequence
from decimal import Decimal
from typing import Literal

from pydantic import BaseModel, Field

from goodfetch_foxhound import AsyncCrawl, AsyncCrawlContext, AsyncPage, JSONValue

# `page.extract` is ADR 0091's; a crawl's Labels, handlers and `enqueue` are ADR 0093's (snippet_crawl.py), its Datasets
# and `push` ADR 0103's (snippet_output.py).


# Scenario 02: a typed model; Pydantic writes the Decimal as an exact string on the wire, never a float.
class Book(BaseModel):
    title: str
    price: Decimal = Field(decimal_places=2)
    in_stock: int
    rating: Literal[1, 2, 3, 4, 5]
    description: str


async def typed_extract(page: AsyncPage) -> str:
    book = (await page.extract("the book on this page", Book)).output  # checked by JSON Schema, then by Pydantic, in the retry loop
    return f"{book.title}: £{book.price}"


async def list_of_models(page: AsyncPage) -> Sequence[Book]:
    return (await page.extract("every book on this page", list[Book])).output  # any type TypeAdapter accepts


async def raw_schema(page: AsyncPage) -> JSONValue:
    # The library-free way down: a hand-written JSON Schema gives a JSONValue to narrow.
    return (await page.extract("the price", {"type": "object", "properties": {"price": {"type": "string"}}})).output


# Scenario 05: the category name travels as a typed Payload, apart from the engine's own fields.
class BookPayload(BaseModel):
    category: str = Field(min_length=1)


class BookRecord(Book):
    url: str
    category: str


crawl = AsyncCrawl()
book_page = crawl.label("book", payload=BookPayload)
books = crawl.dataset("books", schema=BookRecord)


async def on_category_page(ctx: AsyncCrawlContext[None], category_name: str, book_urls: Sequence[str]) -> None:
    for url in book_urls:
        await ctx.enqueue(url, book_page, BookPayload(category=category_name))
    # A plain dict with a wrong field is a type error, not a surprise in the handler.
    await ctx.enqueue("https://books.toscrape.com/x", book_page, {"name": category_name})  # type: ignore[misc]  # pyright: ignore[reportCallIssue, reportArgumentType]


@crawl.handle(book_page)
async def on_book_page(ctx: AsyncCrawlContext[BookPayload]) -> None:
    book = (await ctx.page.extract("the book on this page", Book)).output
    await ctx.push(books, BookRecord(**book.model_dump(), url=ctx.page.url, category=ctx.request.payload.category))
