"""Crawl output in code (ADR 0103, decided in #479).

A crawl declares its Datasets as it declares its Labels, each a typed handle (ADR 0082), with no default Dataset. A
handler pushes Records through its context, which stages them with its attempt: they commit with it, numbered in commit
order, or are discarded when it ends early (ADR 0003). A program reads committed Records back from the crawl's Run as a
sequence that starts at the Dataset's first Record, follows the Run live, and never slows the crawl, since Records are
already in run state. Sinks are settings (`sinks.<name>.*`); an Export is a file the engine writes, which the program
saves or streams wherever the engine runs. Every Run, Collie's included, has a Key-value store; Datasets, Records,
Exports and Sinks are Foxhound's (ADR 0084).
"""

from collections.abc import AsyncIterator, Iterator, Sequence
from dataclasses import dataclass
from datetime import datetime
from os import PathLike
from typing import Generic, Literal, TypeAlias, TypeVar, final, overload

from goodfetch._concurrency import AsyncEventIterator, EventIterator
from goodfetch._data import JsonSchema, JsonValue, SchemaIssue
from goodfetch._results import ProgramError
from goodfetch._time import Timeout

T = TypeVar("T")

@final
@dataclass(frozen=True, kw_only=True)
class DatasetRecord(Generic[T]):
    """One committed Record, as a program reads it back."""

    number: int
    """Its number in its Dataset: 1 for the first, in commit order, with no gaps."""
    value: T
    """Built by the schema's own parse (ADR 0082)."""
    request_id: str
    """The crawl request whose attempt committed it."""
    url: str
    committed_at: datetime

@final
@dataclass(frozen=True, kw_only=True)
class RejectedRecord:
    """A Record a "Record before commit" Hook rejected (ADR 0033), or that a Hook's transform left outside the schema."""

    record: JsonValue
    """As JSON, never parsed by the schema it failed (G3)."""
    reason: str
    hook: str | None
    """The Hook that rejected it, or None when the schema did, after a Hook's transform."""
    request_id: str
    url: str

ExportFormat: TypeAlias = Literal[
    "json", "jsonl", "csv", "xlsx", "xml", "parquet", "html_table", "rss", "atom", "avro", "arrow_ipc", "markdown"
]
"""The Export formats of ADR 0026, named, never inferred from a file name. Each format's own options are `export.*`
settings (area 6)."""

class _ExportInfo:
    @property
    def format(self) -> ExportFormat: ...
    @property
    def records(self) -> int: ...
    @property
    def last_number(self) -> int:
        """The number of the last Record it holds: the Dataset as it stood when the call was made."""
    @property
    def size(self) -> int:
        """In bytes."""
    @property
    def sha256(self) -> str: ...

@final
class AsyncExport(_ExportInfo):
    """A file the engine wrote from a Dataset's Records, wherever the engine runs (ADR 0001). Playwright's names."""

    async def save_as(self, path: str | PathLike[str], *, timeout: Timeout = ...) -> None:
        """Copies it to a path on the program's machine."""
    def stream(self) -> AsyncIterator[bytes]:
        """Its bytes, read once."""

@final
class Export(_ExportInfo):
    def save_as(self, path: str | PathLike[str], *, timeout: Timeout = ...) -> None: ...
    def stream(self) -> Iterator[bytes]: ...

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
    """

    @overload
    async def get(self, key: str, *, timeout: Timeout = ...) -> JsonValue | None:
        """The JSON value under the key, or None when there is none: an answer, never an error."""
    @overload
    async def get(self, key: str, schema: type[T], *, timeout: Timeout = ...) -> T | None:
        """Built by the schema's own parse; one it rejects raises `ValueInvalidError`."""
    @overload
    async def get(self, key: str, schema: JsonSchema, *, timeout: Timeout = ...) -> JsonValue | None: ...
    async def set(self, key: str, value: object, *, timeout: Timeout = ...) -> None:
        """A JSON value or a Pydantic model, dumped in JSON mode; anything else raises `ValueInvalidError`."""
    async def get_bytes(self, key: str, *, timeout: Timeout = ...) -> StoredBytes | None: ...
    async def set_bytes(self, key: str, content: bytes, content_type: str, *, timeout: Timeout = ...) -> None: ...
    async def delete(self, key: str, *, timeout: Timeout = ...) -> bool:
        """Whether the key existed."""
    def keys(self, *, prefix: str = "") -> AsyncEventIterator[str]: ...

@final
class KeyValueStore:
    @overload
    def get(self, key: str, *, timeout: Timeout = ...) -> JsonValue | None: ...
    @overload
    def get(self, key: str, schema: type[T], *, timeout: Timeout = ...) -> T | None: ...
    @overload
    def get(self, key: str, schema: JsonSchema, *, timeout: Timeout = ...) -> JsonValue | None: ...
    def set(self, key: str, value: object, *, timeout: Timeout = ...) -> None: ...
    def get_bytes(self, key: str, *, timeout: Timeout = ...) -> StoredBytes | None: ...
    def set_bytes(self, key: str, content: bytes, content_type: str, *, timeout: Timeout = ...) -> None: ...
    def delete(self, key: str, *, timeout: Timeout = ...) -> bool: ...
    def keys(self, *, prefix: str = "") -> EventIterator[str]: ...

# The Error codes of crawl output, all in the `program` group and none retryable (ADR 0103).

@final
@dataclass(frozen=True, kw_only=True)
class DatasetDetails:
    dataset: str

@final
class DatasetDuplicateError(ProgramError):
    """A second Dataset of one name in one crawl. Raised at once, with no I/O."""

    @property
    def details(self) -> DatasetDetails: ...

@final
class DatasetUnknownError(ProgramError):
    """A push to a Dataset another crawl declared."""

    @property
    def details(self) -> DatasetDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class DatasetKeyInvalidDetails:
    dataset: str
    fields: Sequence[str]

@final
class DatasetKeyInvalidError(ProgramError):
    """A `key` naming a field the schema does not require. Raised when the Dataset is declared."""

    @property
    def details(self) -> DatasetKeyInvalidDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class RecordInvalidDetails:
    dataset: str
    issues: Sequence[SchemaIssue]
    number: int | None
    """The Record's number when reading one back failed its schema's parse; None on a push."""

@final
class RecordInvalidError(ProgramError):
    """A pushed Record failed its Dataset's schema, natively or in the engine, could not be dumped as JSON, or lacked its
    key's fields; it was not staged. Caught, the attempt goes on; escaping the handler, the request ends `failed` with
    it, not retried. Also raised by a reader whose native parse rejects a Record."""

    @property
    def details(self) -> RecordInvalidDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class ValueInvalidDetails:
    key: str
    problem: Literal["not_json", "schema", "bytes"] | str
    issues: Sequence[SchemaIssue]

@final
class ValueInvalidError(ProgramError):
    """A Key-value store value that is not JSON (`set`), fails its schema (`get`), or holds bytes (`get`; `fix` names
    `get_bytes`)."""

    @property
    def details(self) -> ValueInvalidDetails: ...
