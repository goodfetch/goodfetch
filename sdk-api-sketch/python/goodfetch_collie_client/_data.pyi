"""Typed data: schemas, Records and Payloads (decided in #469).

On the wire every schema is JSON Schema and every value is JSON (ADR 0038). A program writes its schemas as a Pydantic
v2 model class, or any class Pydantic's `TypeAdapter` accepts (`list[Book]`, a dataclass, a `TypedDict`), or as a JSON
Schema document by hand, whose values are `JSONValue` (G7, G8). A union or an `Annotated` type goes inside a model
until PEP 747's `TypeForm` can type it as an argument. Pydantic's supported major range is stated in the release notes:
a new major is added in a minor, and dropping one follows ADR 0030's rule for runtimes. Python has no separate input
type: a model is both what a program sends and what it receives, dumped in JSON mode on the wire.

Goodfetch's own result models keep the fields a newer engine adds in `model_extra` and are built without validation,
so nothing new raises (G3); their input models forbid unknown fields.
"""

from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import TypeAlias, TypeVar, final

from goodfetch_collie_client._results import ProgramError

JSONValue: TypeAlias = None | bool | int | float | str | Sequence["JSONValue"] | Mapping[str, "JSONValue"]
"""A JSON value: Goodfetch's own type for data without a declared schema."""

JSONObject: TypeAlias = Mapping[str, JSONValue]

JSONSchema: TypeAlias = Mapping[str, JSONValue]
"""A JSON Schema document written by hand: the library-free way down."""

T = TypeVar("T")
P = TypeVar("P")

@final
@dataclass(frozen=True, kw_only=True)
class SchemaIssue:
    """One problem a validator found, at a path inside the value."""

    message: str
    path: Sequence[str | int]

# The Error codes of typed data, all in the `program` group (ADR 0079).

@final
@dataclass(frozen=True, kw_only=True)
class SchemaValidatorFailedDetails:
    schema: str | None
    """The schema's JSON Schema `title` (a Pydantic model's name), or None when it has none."""
    path: Sequence[str | int]
    """Where in the value the validator was when it raised, when known."""
    exception_type: str
    exception_message: str

@final
class SchemaValidatorFailedError(ProgramError):
    """The program's own validator raised something other than Pydantic's `ValidationError`, such as a `KeyError` in a
    `@field_validator`. A rejection is the call's own failure, or, in an extraction, goes back to the model for another
    attempt (ADR 0012); this stops the call at once.
    `__cause__` holds the original."""

    @property
    def details(self) -> SchemaValidatorFailedDetails: ...
