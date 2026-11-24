"""Schemas in Python's own types, at the seam a typed call goes through (ADR 0082).

A Pydantic model, any type `TypeAdapter` accepts (`list[Book]`, a
dataclass) and a JSON Schema written by hand each give the engine the side
of the schema it parses from, encode a value as it parses back, and decode
an answer to the output type. Only JSON values cross: a value that is not
JSON, or one that would read back as another value, is refused, naming its
field path, and keys pass untouched. The Pydantic cases run at the floor
of the supported range, which `uv.lock` pins. TypeScript's
`typed-data.test.ts` and `json.test.ts` are its twins; the Key-value round
trip is in `goodfetch_collie`'s `test_key_value_store_attached.py`.
"""

import json
import math
import os
from collections.abc import Callable
from dataclasses import dataclass
from datetime import date
from decimal import Decimal
from typing import Any

import pytest
from goodfetch_collie_client.generated.errors import SchemaValidatorFailedError
from goodfetch_collie_client.json_values import JSONObject, JSONValue
from goodfetch_collie_client.schema import (
    JSONSchema,
    SchemaCodec,
    SchemaRejectedError,
    ValueNotJSONError,
    schema_codec,
)
from hypothesis import example, given, seed, settings
from hypothesis import strategies as st
from pydantic import BaseModel, Field, computed_field, field_serializer, field_validator
from typing_extensions import TypeForm


class Book(BaseModel):
    """A program's model: an alias, an exact price and a computed field."""

    title: str
    published_in: int = Field(alias="publishedIn")
    price: Decimal

    def _label(self) -> str:
        """Computed from the title: never sent, since nothing parses it back."""
        return self.title.upper()

    # The call form: mypy refuses a decorator above `@property`.
    label = computed_field(property(_label))


@dataclass
class Reading:
    """A dataclass whose date travels as the ISO string it parses from."""

    book: str
    read_on: date


class Rated(BaseModel):
    """A model whose float a program may set to NaN."""

    rating: float


class Anything(BaseModel):
    """A model with a field Pydantic does not type."""

    extra: Any


class Notes(BaseModel):
    """A model holding a program's own data under any keys."""

    entries: JSONObject


class Pickled:
    """An object only pickle could carry."""


class Shelved(BaseModel):
    """A model whose own validator has a bug: it raises, never rejects."""

    title: str

    @field_validator("title")
    @classmethod
    def _shelf(cls, title: str) -> str:
        shelves: dict[str, str] = {}
        return shelves[title]


class Wrapping(BaseModel):
    """A model whose serializer wraps its field in a fresh object at each dump."""

    held: str

    @field_serializer("held")
    def _wrap(self, held: str) -> dict[str, object]:
        return {"wrapped": Pickled(), "held": held}


DUNE = Book(title="Dune", publishedIn=1965, price=Decimal("9.990"))
DUNE_WIRE: JSONObject = {"title": "Dune", "publishedIn": 1965, "price": "9.990"}
READING = Reading(book="Dune", read_on=date(2026, 1, 2))
READING_WIRE: JSONObject = {"book": "Dune", "read_on": "2026-01-02"}

HAND_WRITTEN: JSONObject = {
    "type": "object",
    "properties": {"title": {"type": "string"}},
    "required": ["title"],
}

PYDANTIC = pytest.mark.covers("story:11-69")
"""A Pydantic case, run at the supported range's floor."""

ROUND_TRIPS = pytest.mark.parametrize(
    ("schema", "value", "wire"),
    [
        pytest.param(Book, DUNE, DUNE_WIRE, marks=PYDANTIC),
        pytest.param(list[Book], [DUNE], [DUNE_WIRE], marks=PYDANTIC),
        pytest.param(Reading, READING, READING_WIRE, marks=PYDANTIC),
        (
            HAND_WRITTEN,
            {"title": "Dune", "year": 1965},
            {"title": "Dune", "year": 1965},
        ),
    ],
    ids=["model", "list-of-models", "dataclass", "hand-written"],
)
"""Each kind of schema, a value of it and the JSON the engine reads."""


def not_json(value: object) -> ValueNotJSONError:
    """What encoding `value` under a hand-written schema raises."""
    with pytest.raises(ValueNotJSONError) as refused:
        schema_codec(HAND_WRITTEN).encode(value)  # type: ignore[arg-type]
    return refused.value


def refused_by(schema: TypeForm[object], value: object) -> tuple[str, str]:
    """Where and what encoding `value` under `schema` refuses as not JSON."""
    with pytest.raises(ValueNotJSONError) as refused:
        schema_codec(schema).encode(value)
    return (refused.value.path, refused.value.found)


def decoded_pointer(path: str) -> str:
    """The key a one-token JSON Pointer names (RFC 6901, section 4)."""
    return path.removeprefix("/").replace("~1", "/").replace("~0", "~")


@pytest.mark.covers("story:11-66")
@ROUND_TRIPS
def test_each_schema_encodes_a_value_as_the_json_it_parses_back(
    schema: JSONSchema | TypeForm[object], value: object, wire: JSONValue
) -> None:
    """By alias, a `Decimal` as an exact string, a date in ISO form, no label."""
    codec: SchemaCodec[Any] = schema_codec(schema)
    assert codec.encode(value) == wire


@pytest.mark.covers("story:11-66")
@ROUND_TRIPS
def test_each_schema_decodes_an_answer_to_its_output_type(
    schema: JSONSchema | TypeForm[object], value: object, wire: JSONValue
) -> None:
    """The schema's own parse builds the Books, the `Decimal` and the `date`."""
    assert schema_codec(schema).decode(wire) == value


@pytest.mark.covers("story:11-66")
@PYDANTIC
def test_a_models_wire_schema_is_its_validation_side_by_alias() -> None:
    """The engine reads `publishedIn`, a price as number or string, and no label."""
    properties = schema_codec(Book).wire_schema["properties"]

    assert properties == {
        "title": {"title": "Title", "type": "string"},
        "publishedIn": {"title": "Publishedin", "type": "integer"},
        "price": {"anyOf": [{"type": "number"}, {"type": "string"}], "title": "Price"},
    }


@pytest.mark.covers("story:11-66")
@PYDANTIC
def test_a_list_of_models_wire_schema_is_an_array_of_them() -> None:
    """`list[Book]` is any type `TypeAdapter` accepts."""
    items = schema_codec(list[Book]).wire_schema["items"]

    assert items == {"$ref": "#/$defs/Book"}


@pytest.mark.covers("story:11-66")
@PYDANTIC
def test_a_dataclass_wire_schema_reads_its_date_as_an_iso_string() -> None:
    """The date's input form is the string it parses from."""
    properties = schema_codec(Reading).wire_schema["properties"]

    assert properties == {
        "book": {"title": "Book", "type": "string"},
        "read_on": {"format": "date", "title": "Read On", "type": "string"},
    }


@pytest.mark.covers("story:11-66")
def test_a_hand_written_json_schema_is_the_wire_schema_as_written() -> None:
    """The way down (G8): the document goes as it is."""
    assert schema_codec(HAND_WRITTEN).wire_schema == HAND_WRITTEN


@pytest.mark.covers("story:11-66")
@PYDANTIC
def test_a_value_the_schema_would_not_parse_back_is_refused_before_sending() -> None:
    """A model built without validation, its year a word, is never sent."""
    unchecked = Book.model_construct(title="Dune", published_in="soon", price=1)

    with pytest.raises(SchemaRejectedError) as refused:
        schema_codec(Book).encode(unchecked)

    assert [issue.path for issue in refused.value.issues] == ["/publishedIn"]


@pytest.mark.covers("story:11-66")
@PYDANTIC
def test_an_answer_of_an_older_shape_fails_the_typed_read_at_its_path() -> None:
    """A stored year as a word fails the schema's own parse, by its wire name."""
    with pytest.raises(SchemaRejectedError) as refused:
        schema_codec(list[Book]).decode(
            [{"title": "Dune", "publishedIn": "soon", "price": "1"}]
        )

    assert [issue.path for issue in refused.value.issues] == ["/0/publishedIn"]


@pytest.mark.covers("story:11-67", "error:schema_validator_failed")
@PYDANTIC
@pytest.mark.parametrize(
    ("schema", "value", "title"),
    [
        (Shelved, {"title": "Dune"}, "Shelved"),
        (list[Shelved], [{"title": "Dune"}], None),
    ],
    ids=["titled", "untitled"],
)
def test_a_validator_that_raises_stops_the_parse_at_once_with_the_original_as_cause(
    schema: TypeForm[object], value: JSONValue, title: str | None
) -> None:
    """Only Pydantic's `ValidationError` is a rejection; a `KeyError` is a bug."""
    with pytest.raises(SchemaValidatorFailedError) as failed:
        schema_codec(schema).decode(value)

    assert failed.value.code == "schema_validator_failed"
    assert failed.value.retryable is False
    assert failed.value.details == {
        "schema": title,
        "path": None,
        "exception_type": "KeyError",
        "exception_message": "'Dune'",
    }
    assert isinstance(failed.value.__cause__, KeyError)
    assert "KeyError: 'Dune'" in str(failed.value)


@pytest.mark.covers("story:11-67", "error:schema_validator_failed")
@PYDANTIC
def test_a_validator_that_raises_stops_a_value_before_it_is_sent() -> None:
    """The parse back of a value to send raises the same way, so nothing is sent."""
    with pytest.raises(SchemaValidatorFailedError) as failed:
        schema_codec(Shelved).encode(Shelved.model_construct(title="Dune"))

    assert isinstance(failed.value.__cause__, KeyError)


@pytest.mark.covers("story:11-68")
@pytest.mark.parametrize(
    ("value", "path", "found"),
    [
        ({"price": {"amount": math.nan}}, "/price/amount", "NaN"),
        ([1, math.inf], "/1", "Infinity"),
        ({"items": [1, Pickled()]}, "/items/1", "an instance of Pickled"),
        ({"counts": {1: "one"}}, "/counts", "the key 1, not a string"),
        ({"a/b~c": (1, 2)}, "/a~1b~0c", "an instance of tuple"),
        (Decimal("1.5"), "", "an instance of Decimal"),
    ],
    ids=["nan", "infinity", "pickle-only", "non-string-key", "pointer-escaped", "root"],
)
def test_a_value_that_is_not_json_is_refused_naming_its_path(
    value: object, path: str, found: str
) -> None:
    """Nothing is turned into a string or dropped on the way (G3)."""
    refused = not_json(value)

    assert (refused.path, refused.found) == (path, found)


@pytest.mark.covers("story:11-68")
def test_a_cycle_is_refused_where_it_closes() -> None:
    """A list that holds itself has no JSON form."""
    cycle: list[object] = [1]
    cycle.append(cycle)

    refused = not_json(cycle)

    assert (refused.path, refused.found) == ("/1", "a cycle")


@pytest.mark.covers("story:11-68")
def test_a_models_nan_is_refused_by_its_wire_path() -> None:
    """Pydantic's JSON dump keeps a float's NaN, which JSON has no form for."""
    assert refused_by(Rated, Rated(rating=math.nan)) == ("/rating", "NaN")


@pytest.mark.covers("story:11-68")
def test_a_pickle_only_object_in_a_model_is_refused_by_its_wire_path() -> None:
    """Pydantic's JSON dump has no encoding for it, and none is invented."""
    value = Anything(extra={"k": [1, Pickled()]})

    assert refused_by(Anything, value) == ("/extra/k/1", "an instance of Pickled")


@pytest.mark.covers("story:11-68")
def test_an_object_a_serializer_makes_afresh_is_refused_by_its_wire_path() -> None:
    """Its dump holds a new instance each time, so it is found by its type."""
    value = Wrapping(held="Dune")

    assert refused_by(Wrapping, value) == ("/held/wrapped", "an instance of Pickled")


@pytest.mark.covers("story:11-68")
@pytest.mark.parametrize(
    ("extra", "path", "found"),
    [
        ({"k": math.nan}, "/extra/k", "NaN"),
        ({"k": Decimal("1.5")}, "/extra/k", "an instance of Decimal"),
        ({1: "one"}, "/extra", "the key 1, not a string"),
        ([(1, 2)], "/extra/0", "an instance of tuple"),
        ({"k": b"ab"}, "/extra/k", "an instance of bytes"),
        ({"k": date(2026, 1, 2)}, "/extra/k", "an instance of date"),
    ],
    ids=["nan", "decimal", "non-string-key", "tuple", "bytes", "date"],
)
def test_a_value_an_untyped_field_would_read_back_changed_is_refused(
    extra: object, path: str, found: str
) -> None:
    """Under `Any`, Pydantic's dump would send null, "1.5", "1" or a list."""
    assert refused_by(Anything, Anything(extra=extra)) == (path, found)


JSON_KEYS = st.one_of(st.text(), st.sampled_from(["__proto__", "constructor", "a/b~c"]))
JSON_VALUES: st.SearchStrategy[JSONValue] = st.recursive(
    st.none()
    | st.booleans()
    | st.integers()
    | st.floats(allow_nan=False, allow_infinity=False)
    | st.text(),
    lambda children: st.lists(children) | st.dictionaries(JSON_KEYS, children),
    max_leaves=20,
)
JSON_OBJECTS: st.SearchStrategy[JSONObject] = st.dictionaries(JSON_KEYS, JSON_VALUES)

SEED = os.environ.get("GOODFETCH_TEST_SEED")


def property_test(test: Callable[..., None]) -> Callable[..., None]:
    """A Hypothesis test on a fixed seed: its name's, or `GOODFETCH_TEST_SEED`.

    No example database: a failure prints its blob, and the case it found is
    committed as an `@example`, Hypothesis's regression file.
    """
    configured = settings(
        database=None, deadline=None, print_blob=True, derandomize=SEED is None
    )(test)
    return configured if SEED is None else seed(int(SEED))(configured)


@pytest.mark.covers("story:11-68")
@property_test
@given(JSON_VALUES)
@example({"__proto__": {"polluted": True}, "constructor": "kept"})
def test_any_json_value_crosses_unchanged_under_a_hand_written_schema(
    value: JSONValue,
) -> None:
    """`__proto__` and every other key pass as given, through the real encoding."""
    titles = schema_codec(HAND_WRITTEN)

    received = titles.decode(json.loads(json.dumps(titles.encode(value))))

    assert received == value


@pytest.mark.covers("story:11-68")
@property_test
@given(JSON_OBJECTS)
@example({"__proto__": {"polluted": True}, "constructor": "kept"})
def test_a_programs_data_in_a_model_keeps_its_keys_through_a_round_trip(
    entries: JSONObject,
) -> None:
    """A model's field of JSON comes back with the keys it was given."""
    notes = schema_codec(Notes)

    received = notes.decode(
        json.loads(json.dumps(notes.encode(Notes(entries=entries))))
    )

    assert received.entries == entries


@pytest.mark.covers("story:11-68")
@property_test
@given(JSON_OBJECTS, JSON_KEYS)
def test_a_nan_planted_under_any_key_is_named_by_it(
    entries: JSONObject, key: str
) -> None:
    """The path is the key as a JSON Pointer (RFC 6901), whatever it holds."""
    refused = not_json({**entries, key: math.nan})

    assert (decoded_pointer(refused.path), refused.found) == (key, "NaN")
