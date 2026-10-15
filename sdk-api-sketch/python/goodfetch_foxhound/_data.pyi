"""Foxhound's typed data (ADR 0082, decided in #469): the handles on a crawl's Datasets and Labels, and the Payload's
Error codes. Schemas and JSON values are the Collie client package's (ADR 0095)."""

from collections.abc import Sequence
from dataclasses import dataclass
from typing import Generic, TypeVar, final

from goodfetch_collie_client import ProgramError, SchemaIssue

T = TypeVar("T")
P = TypeVar("P")

class Dataset(Generic[T]):
    """A handle on a crawl's Dataset of `T`, the schema it was declared with, or `JSONObject` when it declares none.

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

