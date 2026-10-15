"""A Run's Key-value store (ADR 0103): every Run keeps one, Collie's included (ADR 0088). Datasets, Records, Exports
and Sinks are Foxhound's, in `goodfetch_foxhound` (ADR 0095).
"""

from collections.abc import Sequence
from dataclasses import dataclass
from typing import Literal, TypeVar, final, overload

from goodfetch_collie_client._concurrency import AsyncEventIterator, EventIterator
from goodfetch_collie_client._data import JSONSchema, JSONValue, SchemaIssue
from goodfetch_collie_client._results import ProgramError
from goodfetch_collie_client._time import Timeout

T = TypeVar("T")

"""The Export formats of ADR 0026, named, never inferred from a file name. Each format's own options are `export.*`
settings (area 6)."""

@final
@dataclass(frozen=True, kw_only=True)
class StoredBytes:
    """Bytes kept under a key, with their content type."""

    content: bytes
    content_type: str

@final
class AsyncKeyValueStore:
    """A Run's Key-value store: files, blobs and the program's own state, in the run file, checkpointed with it.

    Reached from the program's Run, it writes at once. Reached through a handler's `ctx.run`, which is bound to its attempt
    (ADR 0093), it stages its writes with the attempt, which commit or are discarded with its Records and enqueues, and
    its reads see the attempt's own writes first.

    For each key the last commit wins. A read sees the committed values and its own attempt's staged writes; another
    attempt's staged writes stay invisible until it commits. There is no compare-and-set: a value concurrent handlers
    share live stays in the program's own memory, within one process; across worker processes a program counts from
    committed Records or outcomes.
    """

    @overload
    async def get(self, key: str, *, timeout: Timeout = ...) -> JSONValue | None:
        """The JSON value under the key, or None when there is none: an answer, never an error."""
    @overload
    async def get(self, key: str, schema: type[T], *, timeout: Timeout = ...) -> T | None:
        """Built by the schema's own parse; one it rejects raises `ValueInvalidError`."""
    @overload
    async def get(self, key: str, schema: JSONSchema, *, timeout: Timeout = ...) -> JSONValue | None: ...
    async def set(self, key: str, value: object, *, timeout: Timeout = ...) -> None:
        """A JSON value or a Pydantic model, dumped in JSON mode; `None` or anything else raises `ValueInvalidError`
        (`fix` names `delete` for `None`), so `None` from `get` always means absent."""
    async def get_bytes(self, key: str, *, timeout: Timeout = ...) -> StoredBytes | None: ...
    async def set_bytes(self, key: str, content: bytes, content_type: str, *, timeout: Timeout = ...) -> None: ...
    async def delete(self, key: str, *, timeout: Timeout = ...) -> bool:
        """Whether the key existed."""
    def keys(self, *, prefix: str = "") -> AsyncEventIterator[str]: ...

@final
class KeyValueStore:
    @overload
    def get(self, key: str, *, timeout: Timeout = ...) -> JSONValue | None: ...
    @overload
    def get(self, key: str, schema: type[T], *, timeout: Timeout = ...) -> T | None: ...
    @overload
    def get(self, key: str, schema: JSONSchema, *, timeout: Timeout = ...) -> JSONValue | None: ...
    def set(self, key: str, value: object, *, timeout: Timeout = ...) -> None: ...
    def get_bytes(self, key: str, *, timeout: Timeout = ...) -> StoredBytes | None: ...
    def set_bytes(self, key: str, content: bytes, content_type: str, *, timeout: Timeout = ...) -> None: ...
    def delete(self, key: str, *, timeout: Timeout = ...) -> bool: ...
    def keys(self, *, prefix: str = "") -> EventIterator[str]: ...

# The Key-value store's Error code, in the `program` group and not retryable (ADR 0103).

@final
@dataclass(frozen=True, kw_only=True)
class ValueInvalidDetails:
    key: str
    problem: Literal["not_json", "null", "schema", "bytes"] | str
    issues: Sequence[SchemaIssue]

@final
class ValueInvalidError(ProgramError):
    """A Key-value store value that is not JSON or is `None` (`set`; `fix` names `delete`), fails its schema (`get`),
    or holds bytes (`get`; `fix` names `get_bytes`)."""

    @property
    def details(self) -> ValueInvalidDetails: ...
