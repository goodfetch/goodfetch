"""Typed data: schemas, Records and Payloads (decided in #469).

On the wire every schema is JSON Schema and every value is JSON (ADR 0038). A program writes its schemas as a Pydantic
v2 model class, or any class Pydantic's `TypeAdapter` accepts (`list[Book]`, a dataclass, a `TypedDict`), or as a JSON
Schema document by hand, whose values are `JsonValue` (G7, G8). A union or an `Annotated` type goes inside a model
until PEP 747's `TypeForm` can type it as an argument. Pydantic's supported major range is stated in the release notes:
a new major is added in a minor, and dropping one follows ADR 0030's rule for runtimes. Python has no separate input
type: a model is both what a program sends and what it receives, dumped in JSON mode on the wire.

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
    """A handle on a crawl's Dataset of `T`, the schema it was declared with, or `JsonObject` when it declares none.

    The engine validates every Record against the schema's JSON Schema. A crawl declares it with `crawl.dataset()`, a
    handler pushes with `ctx.push()`, and a program reads with `run.records()` (ADR 0103, `_output`). A value the model
    cannot dump as JSON raises `RecordInvalidError`; nothing is ever turned into a string or pickled.
    """

    @property
    def name(self) -> str: ...
    @property
    def key(self) -> Sequence[str]:
        """The fields that identify a Record (ADR 0103); empty when it declares none."""

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
    """Empty when the Payload was refused for its size; the error's `limit` then names `crawl.payload.max_size`."""
    size_bytes: int
    """The Payload's size as serialized JSON."""

@final
class PayloadInvalidError(ProgramError):
    """A Payload failed its Label's schema, was not JSON, or exceeded `crawl.payload.max_size`. Raised by the enqueue
    call, or the failure of a request whose handler's parse rejected a Payload that a changed native rule no longer
    accepts."""

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
