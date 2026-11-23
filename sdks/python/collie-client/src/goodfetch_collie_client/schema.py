"""Schemas in Python's own types (ADR 0082, G7): how one crosses the wire.

A schema is a Pydantic v2 model class, any other type Pydantic's
`TypeAdapter` accepts (`list[Book]`, a dataclass, a `TypedDict`), or a
JSON Schema written by hand, the way down (G8), whose values are
`JSONValue`. The SDK uses only Pydantic's public API. A typed call goes
through `schema_codec`: the engine receives the schema's validation side,
a sent value is encoded as it parses back (by alias, a `Decimal` as an
exact string, no computed field), and an answer is decoded to the output
type by the schema's own parse. Pydantic has one type for both. The
codec is typed by the schema as `TypeAdapter` is, through a `TypeForm`:
a `Literal`, an `Annotated` or a union types it too.

Each call names its own refusals (`value_invalid`, `payload_invalid`, a
Record's), so the codec raises `ValueNotJSONError` and
`SchemaRejectedError`, which the call turns into its code. Only Pydantic's
`ValidationError` is a rejection: anything else a validator raises stops
the call at once with `schema_validator_failed`, the same for every call,
the original as its cause. TypeScript's `schema.ts` is its twin.
"""

from __future__ import annotations

import json
from abc import ABC, abstractmethod
from dataclasses import dataclass
from typing import TYPE_CHECKING, Generic, TypeAlias, TypeVar, cast, overload

from pydantic import TypeAdapter, ValidationError
from typing_extensions import TypeForm, TypeIs

from goodfetch_collie_client.generated.errors import SchemaValidatorFailedError
from goodfetch_collie_client.json_values import (
    JSONObject,
    JSONValue,
    NonJSON,
    non_json,
    pointer_token,
)

if TYPE_CHECKING:
    from collections.abc import Callable, Iterable

__all__ = ["JSONSchema", "SchemaCodec", "schema_codec"]

T = TypeVar("T")

JSONSchema: TypeAlias = JSONObject
"""A JSON Schema document written by hand, the way down (G8)."""


@dataclass(frozen=True)
class SchemaIssue:
    """One issue the schema's own parse found, at its JSON Pointer."""

    path: str
    """The JSON Pointer (RFC 6901) of the value it is about, empty at the root."""
    message: str
    """Pydantic's own message."""


class ValueNotJSONError(ValueError):
    """A value to send holds something that is not JSON (G3)."""

    def __init__(self, refused: NonJSON) -> None:
        """Takes where the value that is not JSON sits, and what it is."""
        super().__init__(f"{refused.found} at {refused.path or 'its root'} is not JSON")
        self.path = refused.path
        """Its JSON Pointer, empty at the root."""
        self.found = refused.found
        """What it is: `NaN`, `an instance of Book`, a key not a string."""


class SchemaRejectedError(ValueError):
    """The schema's own parse rejected a value: one sent, as it parses back, or read."""

    def __init__(self, issues: tuple[SchemaIssue, ...]) -> None:
        """Takes each issue, by its path."""
        super().__init__(
            "; ".join(
                f"{issue.path or 'its root'}: {issue.message}" for issue in issues
            )
        )
        self.issues = issues
        """Each issue the parse found."""


class SchemaCodec(ABC, Generic[T]):
    """How values of one schema cross the wire. A program never constructs one."""

    @property
    @abstractmethod
    def wire_schema(self) -> JSONObject:
        """The JSON Schema the engine receives: the side the schema parses from."""

    @abstractmethod
    def encode(self, value: T) -> JSONValue:
        """`value` as JSON, in the form the schema parses back.

        Raises:
            ValueNotJSONError: Something in it is not JSON.
            SchemaRejectedError: The schema would not parse its JSON back.
            SchemaValidatorFailedError: A validator raised something other
                than a rejection.
        """

    @abstractmethod
    def decode(self, value: JSONValue) -> T:
        """`value`, an answer, parsed by the schema into its output type.

        Raises:
            SchemaRejectedError: The schema rejects it.
            SchemaValidatorFailedError: A validator raised something other
                than a rejection.
        """


@overload
def schema_codec(schema: JSONSchema) -> SchemaCodec[JSONValue]: ...
@overload
def schema_codec(schema: TypeForm[T]) -> SchemaCodec[T]: ...
def schema_codec(
    schema: JSONSchema | TypeForm[T],
) -> SchemaCodec[JSONValue] | SchemaCodec[T]:
    """The codec of `schema`: a type Pydantic adapts, or a hand-written JSON Schema.

    Raises:
        ValueNotJSONError: A hand-written schema holds something that is not JSON.
        pydantic.PydanticSchemaGenerationError: Pydantic cannot adapt the type.
    """
    if _is_hand_written(schema):
        return _HandWritten(schema)
    return _Typed(TypeAdapter(schema))


def _is_hand_written(schema: object) -> TypeIs[JSONSchema]:
    """Whether `schema` is a JSON Schema object rather than a type."""
    return isinstance(schema, dict)


class _HandWritten(SchemaCodec[JSONValue]):
    """A JSON Schema written by hand: the program checks nothing, values are JSON."""

    def __init__(self, document: JSONSchema) -> None:
        _raise_non_json(document)
        self._document = document

    @property
    def wire_schema(self) -> JSONObject:
        return self._document

    def encode(self, value: JSONValue) -> JSONValue:
        _raise_non_json(value)
        return value

    def decode(self, value: JSONValue) -> JSONValue:
        return value


class _Typed(SchemaCodec[T]):
    """A type Pydantic adapts, through its public `TypeAdapter`."""

    def __init__(self, adapter: TypeAdapter[T]) -> None:
        self._adapter = adapter
        self._wire_schema = cast(
            "JSONObject", adapter.json_schema(mode="validation", by_alias=True)
        )

    @property
    def wire_schema(self) -> JSONObject:
        return self._wire_schema

    def encode(self, value: T) -> JSONValue:
        try:
            encoded: object = self._adapter.dump_python(
                value,
                mode="json",
                by_alias=True,
                exclude_computed_fields=True,
                warnings=False,
                fallback=_refuse,
            )
        except _UnknownTypeError as unknown:
            raise ValueNotJSONError(self._locate(value, unknown.value)) from None
        _raise_non_json(encoded)
        json_value = cast("JSONValue", encoded)
        # What the engine reads parses back: an alias validation does not
        # read, or a value built without validation, is refused here.
        parsed = self._parse(json_value)
        # And it parses back as the value sent: under `Any`, Pydantic's JSON
        # dump turns a `Decimal` into a string, NaN into null, a key `1`
        # into "1", where a declared field's own type reads them back.
        changed = _first_change(self._dump(value), self._dump(parsed), "")
        if changed is not None:
            raise ValueNotJSONError(changed)
        return json_value

    def decode(self, value: JSONValue) -> T:
        return self._parse(value)

    def _parse(self, value: JSONValue) -> T:
        try:
            return self._adapter.validate_json(json.dumps(value, allow_nan=False))
        except ValidationError as rejected:
            raise SchemaRejectedError(
                tuple(
                    SchemaIssue(path=_pointer(error["loc"]), message=error["msg"])
                    for error in rejected.errors(include_url=False)
                )
            ) from None
        except Exception as raised:
            # Only Pydantic's `ValidationError` is a rejection: anything else
            # a validator raises is the program's bug, so the call stops here.
            raise _validator_failed(self._wire_schema, raised) from raised

    def _dump(self, value: T) -> object:
        """`value` by alias in Python's own types, which keep what it holds."""
        return self._adapter.dump_python(
            value, by_alias=True, exclude_computed_fields=True, warnings=False
        )

    def _locate(self, value: T, unknown: object) -> NonJSON:
        """Where `unknown` sits in `value`'s dump: itself, or a serializer's copy."""
        dumped = self._dump(value)
        path = _path_of(dumped, lambda member: member is unknown, "")
        if path is None:
            path = _path_of(dumped, lambda member: type(member) is type(unknown), "")
        found = f"an instance of {type(unknown).__qualname__}"
        return NonJSON("" if path is None else path, found)


def _validator_failed(
    wire_schema: JSONObject, raised: Exception
) -> SchemaValidatorFailedError:
    """The `schema_validator_failed` of a schema whose validation raised `raised`.

    Its message names the exception's type and message; the details name
    the schema by its JSON Schema `title`, and no path, which an exception
    raised past Pydantic does not carry.
    """
    title = wire_schema.get("title")
    schema = title if isinstance(title, str) else None
    kind = type(raised).__name__
    message = str(raised)
    named = "the schema" if schema is None else f"the schema {schema}"
    return SchemaValidatorFailedError(
        f"{named}'s own validation raised {kind}: {message}",
        fix=(
            "Fix the validator so that it rejects with a ValueError, which "
            "Pydantic reports as a ValidationError, or never raises."
        ),
        details={
            "schema": schema,
            "path": None,
            "exception_type": kind,
            "exception_message": message,
        },
    )


class _UnknownTypeError(Exception):
    """A value Pydantic's JSON dump has no encoding for."""

    def __init__(self, value: object) -> None:
        super().__init__()
        self.value = value


def _refuse(value: object) -> object:
    """Pydantic's fallback for a type it cannot encode: never a string (G3)."""
    raise _UnknownTypeError(value)


def _raise_non_json(value: object) -> None:
    refused = non_json(value)
    if refused is not None:
        raise ValueNotJSONError(refused)


def _path_of(
    tree: object, is_target: Callable[[object], bool], path: str
) -> str | None:
    """The JSON Pointer of the first member of `tree`, depth first, that `is_target`."""
    if is_target(tree):
        return path
    members: Iterable[tuple[object, object]]
    if isinstance(tree, dict):
        members = cast("dict[object, object]", tree).items()
    elif isinstance(tree, (list, tuple, set, frozenset)):
        members = enumerate(cast("Iterable[object]", tree))
    else:
        return None
    for key, member in members:
        found = _path_of(member, is_target, f"{path}/{pointer_token(str(key))}")
        if found is not None:
            return found
    return None


def _first_change(sent: object, read: object, path: str) -> NonJSON | None:
    """Where `read`, the sent value parsed back, first differs from `sent`."""
    kinds = (type(sent), type(read))
    if kinds == (dict, dict):
        return _first_member_change(
            cast("dict[object, object]", sent), cast("dict[object, object]", read), path
        )
    if kinds == (list, list):
        sent_items = cast("list[object]", sent)
        read_items = cast("list[object]", read)
        if len(sent_items) == len(read_items):
            for index, (item, read_item) in enumerate(
                zip(sent_items, read_items, strict=True)
            ):
                changed = _first_change(item, read_item, f"{path}/{index}")
                if changed is not None:
                    return changed
            return None
    elif _same_leaf(sent, read):
        return None
    refused = non_json(sent)
    if refused is not None:
        return NonJSON(path + refused.path, refused.found)
    return NonJSON(path, f"a value that reads back as {read!r}")


def _first_member_change(
    sent: dict[object, object], read: dict[object, object], path: str
) -> NonJSON | None:
    for key, member in sent.items():
        if type(key) is not str:
            return NonJSON(path, f"the key {key!r}, not a string")
        if key not in read:
            return NonJSON(path, f"the key {key!r}, which does not read back")
        changed = _first_change(member, read[key], f"{path}/{pointer_token(key)}")
        if changed is not None:
            return changed
    if sent.keys() != read.keys():
        return NonJSON(path, "keys added when read back")
    return None


def _same_leaf(sent: object, read: object) -> bool:
    """A number reads back as an equal number; anything else as itself, same type."""
    numbers = (int, float)
    if type(sent) in numbers and type(read) in numbers:
        return sent == read
    return type(sent) is type(read) and sent == read


def _pointer(location: tuple[int | str, ...]) -> str:
    """A Pydantic error's location as a JSON Pointer (RFC 6901)."""
    return "".join(f"/{pointer_token(str(part))}" for part in location)
