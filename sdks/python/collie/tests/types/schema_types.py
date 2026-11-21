"""Schemas in Python's own types (ADR 0082), checked by pyright and mypy.

A typed call's codec is typed by its schema: a Pydantic model, `list[Book]`
or a dataclass sends and receives that type, and a JSON Schema written by
hand sends and receives `JSONValue`; a `Literal`, an `Annotated` or a
union types it too, as `TypeAdapter`'s `TypeForm` does. An input with an
unknown field, a value of another type and a value that is not JSON fail
to type-check; mypy's `--strict` reports an ignore that no longer silences an error.
TypeScript's `typed-data.test-d.ts` is its twin.
"""

from dataclasses import dataclass
from datetime import date
from decimal import Decimal
from typing import Annotated, Literal, assert_type

from goodfetch_collie import JSONObject, JSONSchema, JSONValue
from goodfetch_collie_client.generated.models import SampleEchoParams
from goodfetch_collie_client.schema import SchemaCodec, schema_codec
from pydantic import BaseModel, Field


class Book(BaseModel):
    """A program's model."""

    title: str
    price: Decimal


@dataclass
class Reading:
    """A program's dataclass."""

    book: str
    read_on: date


HAND_WRITTEN: JSONSchema = {"type": "object"}


def typed_by_the_schema(answer: JSONValue) -> None:
    """Each codec sends and receives its schema's type."""
    book = Book(title="Dune", price=Decimal("9.99"))
    assert_type(schema_codec(Book), SchemaCodec[Book])
    assert_type(schema_codec(Book).decode(answer), Book)
    assert_type(schema_codec(list[Book]).decode(answer), list[Book])
    assert_type(schema_codec(Reading).decode(answer), Reading)
    assert_type(schema_codec(HAND_WRITTEN).decode(answer), JSONValue)
    assert_type(schema_codec(Book).encode(book), JSONValue)
    assert_type(schema_codec(Book).wire_schema, JSONObject)
    assert_type(
        schema_codec(Literal["sf", "fantasy"]), SchemaCodec[Literal["sf", "fantasy"]]
    )
    assert_type(schema_codec(Annotated[int, Field(gt=0)]), SchemaCodec[int])
    assert_type(schema_codec(Book | None), SchemaCodec[Book | None])
    schema_codec(list[Book]).encode([book])
    schema_codec(HAND_WRITTEN).encode({"title": "Dune", "tags": ["sf"], "year": 1965})


def refused_by_the_checker(book: Book) -> None:
    """A value of another type, or one that is not JSON, does not type-check."""
    schema_codec(Book).encode(Reading(book="Dune", read_on=date(2026, 1, 2)))  # type: ignore[arg-type]
    schema_codec(list[Book]).encode(book)  # type: ignore[arg-type]
    schema_codec(HAND_WRITTEN).encode({"price": Decimal("9.99")})  # type: ignore[dict-item]
    schema_codec(HAND_WRITTEN).encode({1: "one"})  # type: ignore[dict-item]


def inputs_stay_strict() -> None:
    """A generated input model refuses a field it does not declare."""
    SampleEchoParams(text="hi", txet=1)  # type: ignore[call-arg]
