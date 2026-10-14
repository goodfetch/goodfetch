"""Typed data: schemas, Records and Payloads (decided in #469).

On the wire every schema is JSON Schema and every value is JSON (ADR 0038). A program writes its schemas as a Pydantic
v2 model class, or any type Pydantic's `TypeAdapter` accepts (`list[Book]`, a dataclass, a `TypedDict`), or as a JSON
Schema document by hand, whose values are `JsonValue` (G7, G8). Pydantic's supported major range is pinned and stated
in the release notes; narrowing it follows ADR 0030's rule for runtimes. Python has no separate input type: a model is
both what a program sends and what it receives.

Goodfetch's own result models keep the fields a newer engine adds in `model_extra` and are built without validation,
so nothing new raises (G3); their input models forbid unknown fields.
"""

from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import Generic, TypeAlias, TypeVar, final

from goodfetch._results import ProgramError

JsonValue: TypeAlias = None | bool | int | float | str | Sequence["JsonValue"] | Mapping[str, "JsonValue"]
"""A JSON value: Goodfetch's own type for data without a declared schema."""

JsonObject: TypeAlias = Mapping[str, JsonValue]

JsonSchema: TypeAlias = Mapping[str, JsonValue]
"""A JSON Schema document written by hand: the library-free way down."""

T = TypeVar("T")
P = TypeVar("P")

@final
@dataclass(frozen=True, kw_only=True)
class SchemaIssue:
    """One problem a validator found, at a path inside the value."""

    message: str
    path: Sequence[str | int]

class Dataset(Generic[T]):
    """A Run's Dataset of `T`, the schema it was declared with, or `JsonObject` when it declares none.

    The engine validates every Record against the schema's JSON Schema. How a Dataset is declared, how Records are read
    and what becomes of a rejected one are #479's.
    """

    @property
    def name(self) -> str: ...
    async def push(self, record: T) -> None:
        """Provisional: push or pull is #479's. A value that is not JSON, and that the model does not encode, raises
        `ProgramError`; nothing is ever turned into a string or pickled."""

class Label(Generic[P]):
    """A handle on a Label, carrying its Payload's type; `Label[None]` when it has none.

    A program names the destination of an enqueue with this handle, never with a bare string, so a misspelled Label or
    a wrong Payload is a type error. How a crawl declares its Labels and handlers is #477's.
    """

    @property
    def name(self) -> str: ...

# The Error codes of typed data, all in the `program` group (ADR 0079).

@final
@dataclass(frozen=True, kw_only=True)
class PayloadInvalidDetails:
    label: str
    issues: Sequence[SchemaIssue]
    """Empty when the Payload was refused for its size; the error's `limit` then names `crawl.payload.max_bytes`."""
    size_bytes: int
    """The Payload's size as serialized JSON."""

@final
class PayloadInvalidError(ProgramError):
    """A Payload failed its Label's schema, was not JSON, or exceeded `crawl.payload.max_bytes`. Raised by the enqueue
    call."""

    @property
    def details(self) -> PayloadInvalidDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class PayloadSchemaChangedDetails:
    label: str
    failing: int
    """Pending requests of this Label whose Payload fails its current schema."""
    pending: int
    first_issue: SchemaIssue

@final
class PayloadSchemaChangedError(ProgramError):
    """A resume found pending Payloads that fail their Label's changed schema; nothing ran. Fix the schema (a default
    for the new field) or start a new Run."""

    @property
    def details(self) -> PayloadSchemaChangedDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class SchemaValidatorFailedDetails:
    schema_name: str
    """The model or type whose validator raised."""
    path: Sequence[str | int]
    """Where in the value the validator was when it raised, when known."""
    exception_type: str
    exception_message: str

@final
class SchemaValidatorFailedError(ProgramError):
    """The program's own validator raised something other than Pydantic's `ValidationError`, such as a `KeyError` in a
    `@field_validator`. A rejection goes back to the model for another attempt (ADR 0012); this stops the call at once.
    `__cause__` holds the original."""

    @property
    def details(self) -> SchemaValidatorFailedDetails: ...
