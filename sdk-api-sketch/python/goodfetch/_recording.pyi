"""Response recordings (ADR 0105, decided in #482).

A Run's start may name a Response recording: a WARC 1.1 file on the program's machine, which the SDK reads and writes
there, as it does a Plan file (ADR 0091), so a Daemon or Goodfetch Cloud works the same.

- `replay`: every request the Run would send to an address a page can reach (ADR 0017's first class: crawl requests,
  navigations, browser subresources, `page.request`, robots.txt) is answered from the file and never sent. One the file
  lacks is refused: a call the program made raises `ResponseNotRecordedError`, a crawl request ends `failed` with it, and
  a subresource fails in its page as a network error. Every miss is listed in the Run report. Hosts the Run allowlists
  for tests (ADR 0017), such as a test's own server on loopback, are reached as usual.
- `record`: the Run fetches as usual, and the file is replaced, atomically, by every response it received when the Run
  ends. No credential is kept.

The testing kit (`goodfetch.testing`) replays by default. Model calls are never recorded: a Plan replayed with
`plan.frozen` or the kit's Scripted model stands in for the model (ADRs 0065, 0091).
"""

from collections.abc import Sequence
from dataclasses import dataclass
from datetime import datetime
from os import PathLike
from typing import Literal, final

from goodfetch._results import ProgramError, RefusedError

@final
@dataclass(frozen=True, kw_only=True)
class Recording:
    """A Response recording a Run's start names: `replay` it, or `record` into it."""

    mode: Literal["replay", "record"]
    path: str | PathLike[str]
    """A WARC file, relative to the working directory, gzipped per record when it ends in `.gz`."""

@final
@dataclass(frozen=True, kw_only=True)
class RecordingMiss:
    """A request a replaying Run did not find in its recording, and so never sent."""

    method: str
    url: str

@final
@dataclass(frozen=True, kw_only=True)
class RecordingReport:
    """What a Run did with its Response recording, in its report."""

    mode: Literal["replay", "record"] | str
    path: str
    responses: int
    """Responses answered from the file (`replay`) or written to it (`record`)."""
    recorded_from: datetime | None
    """When the responses used were first fetched: the recording's age. `None` when none was used."""
    recorded_to: datetime | None
    recorded_by: str | None
    """The engine that wrote the file."""
    misses: Sequence[RecordingMiss]
    """Every request not found, in order. Empty on `record`."""

@final
@dataclass(frozen=True, kw_only=True)
class ResponseNotRecordedDetails:
    method: str
    url: str
    recording: str
    nearest: Sequence[str]
    """Recorded URLs closest to it (same host and path), for a query string that changed."""

@final
class ResponseNotRecordedError(RefusedError):
    """A replaying Run refused a request its recording lacks, before sending anything. Not retryable."""

    @property
    def details(self) -> ResponseNotRecordedDetails: ...

@final
@dataclass(frozen=True, kw_only=True)
class RecordingInvalidDetails:
    recording: str
    reason: str

@final
class RecordingInvalidError(ProgramError):
    """The recording cannot be read, is not WARC, or names a WARC version this engine does not read (ADR 0030)."""

    @property
    def details(self) -> RecordingInvalidDetails: ...
