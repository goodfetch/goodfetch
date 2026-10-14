"""Snippet for Typed data: schemas, Records and Payloads (#469), on golden scenarios 02 and 05."""

from collections.abc import Sequence
from decimal import Decimal
from typing import Literal, TypeVar, overload

from pydantic import BaseModel, Field

from goodfetch import Dataset, JsonSchema, JsonValue, Label

T = TypeVar("T")
P = TypeVar("P")

# Provisional signatures, each its owner's to decide: `extract` is #473's, a crawl's Labels, handlers and `enqueue` are
# #477's, Datasets are #479's.


@overload
async def extract(instruction: str, schema: type[T]) -> T: ...
@overload
async def extract(instruction: str, schema: JsonSchema) -> JsonValue: ...
async def extract(instruction: str, schema: type[object] | JsonSchema) -> object:
    raise NotImplementedError


async def enqueue(url: str, label: Label[P], payload: P) -> None:
    raise NotImplementedError


def label(name: str, payload: type[P]) -> Label[P]:
    raise NotImplementedError


def dataset(name: str, schema: type[T]) -> Dataset[T]:
    raise NotImplementedError


# Scenario 02: a typed model; Pydantic writes the Decimal as an exact string on the wire, never a float.
class Book(BaseModel):
    title: str
    price: Decimal = Field(decimal_places=2)
    in_stock: int
    rating: Literal[1, 2, 3, 4, 5]
    description: str


async def typed_extract() -> str:
    book = await extract("the book on this page", Book)  # checked by JSON Schema, then by Pydantic, in the retry loop
    return f"{book.title}: £{book.price}"


async def list_of_models() -> Sequence[Book]:
    return await extract("every book on this page", list[Book])  # any type TypeAdapter accepts


async def raw_schema() -> JsonValue:
    # The library-free way down: a hand-written JSON Schema gives a JsonValue to narrow.
    return await extract("the price", {"type": "object", "properties": {"price": {"type": "string"}}})


# Scenario 05: the category name travels as a typed Payload, apart from the engine's own fields.
class BookPayload(BaseModel):
    category: str = Field(min_length=1)


class BookRecord(Book):
    url: str
    category: str


book_page = label("book", BookPayload)
books = dataset("books", BookRecord)


async def on_category_page(category_name: str, book_urls: Sequence[str]) -> None:
    for url in book_urls:
        await enqueue(url, book_page, BookPayload(category=category_name))
    # A plain dict with a wrong field is a type error, not a surprise in the handler.
    await enqueue("https://books.toscrape.com/x", book_page, {"name": category_name})  # type: ignore[misc]  # pyright: ignore[reportArgumentType]


async def on_book_page(url: str, payload: BookPayload) -> None:
    book = await extract("the book on this page", Book)
    await books.push(BookRecord(**book.model_dump(), url=url, category=payload.category))
