"""Crawl output in code (ADR 0103, decided in #479).

A crawl declares its Datasets as it declares its Labels, each a typed handle (ADR 0082), with no default Dataset. A
handler pushes Records through its context, which stages them with its attempt: they commit with it, numbered in commit
order, or are discarded when it ends early (ADR 0003). A program reads committed Records back from the crawl's Run as an
iterator that starts at the Dataset's first Record, follows the Run live, and never slows the crawl. Sinks are settings
(`sinks.<name>.*`, `_settings`); an Export is a file the engine writes, which the program saves or streams wherever the
engine runs. Datasets and Sinks are Foxhound's (ADR 0088); every Run's Key-value store, Collie's included, is the client
package's.
"""

from collections.abc import AsyncIterator, Iterator, Sequence
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Generic, Literal, TypeAlias, TypeVar, final

from goodfetch_collie_client import JSONValue, ProgramError, SchemaIssue, Timeout

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

    number: int
    """Its number in the Dataset's rejection table: 1 for the first, in commit order, with no gaps."""
    record: JSONValue
    """As JSON, never parsed by the schema it failed (G3)."""
    reason: str
    hook: str | None
    """The Hook that rejected it, or None when the schema did, after a Hook's transform."""
    request_id: str
    url: str

ExportFormat: TypeAlias = Literal[
    "json", "jsonl", "csv", "xlsx", "xml", "parquet", "html_table", "rss", "atom", "avro", "arrow_ipc", "markdown"
]
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

    async def save_as(self, path: str | Path, *, timeout: Timeout = ...) -> None:
        """Copies it to a path on the program's machine."""
    def stream(self) -> AsyncIterator[bytes]:
        """Its bytes, read once: an async iterator, not the Node `Readable` of Playwright's `createReadStream()`."""

@final
class Export(_ExportInfo):
    def save_as(self, path: str | Path, *, timeout: Timeout = ...) -> None: ...
    def stream(self) -> Iterator[bytes]: ...

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
@dataclass(frozen=True, kw_only=True)
class DatasetUnknownDetails:
    dataset: str
    records: int
    """The Records it holds, when a resume found it no longer declared; 0 on a push."""

@final
class DatasetUnknownError(ProgramError):
    """A push to a Dataset another crawl declared, or a resume whose crawl no longer declares a Dataset that holds
    Records; nothing ran."""

    @property
    def details(self) -> DatasetUnknownDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class DatasetKeyInvalidDetails:
    dataset: str
    fields: Sequence[str]

@final
class DatasetKeyInvalidError(ProgramError):
    """A `key` naming a field the schema does not require, raised when the Dataset is declared; or a resume whose
    Dataset's key changed."""

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

