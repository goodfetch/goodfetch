"""The types of AI work's result (ADR 0117), checked by pyright and mypy.

The answer sits in how the work ended, so reading `output` before matching
`ended` on `Finished` fails both checkers; a result of a narrower output fits
where a broader one is expected; mypy's `--strict` reports an ignore that no
longer silences an error (story 11-34). TypeScript's `ai-work.test-d.ts` is
its twin.
"""

from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import assert_type

from goodfetch_collie import AIWorkResult, Finished, Stopped
from goodfetch_collie_client.generated.models import StopReason
from goodfetch_collie_client.json_values import JSONValue


@dataclass(frozen=True)
class Author:
    """An output a program's schema types."""

    name: str


def describe(result: AIWorkResult[object]) -> str:
    """A helper that takes any AI work's result."""
    return str(result.verdict)


def reads(result: AIWorkResult[Author]) -> None:
    """The output is read only once `ended` is matched as `Finished`."""
    _ = result.output  # type: ignore[attr-defined]
    _ = result.ended.output  # type: ignore[union-attr]
    match result.ended:
        case Finished(output=output):
            assert_type(output, Author)
        case Stopped(reason=reason, partial=partial):
            assert_type(reason, StopReason)
            assert_type(partial, Mapping[str, JSONValue] | Sequence[JSONValue])
    assert_type(result.require_verified(), Author)
    describe(result)


def widens(finished: Finished[Author], stopped: Stopped[Author]) -> None:
    """How work ended is covariant in its output, as the result is."""
    _: Finished[object] = finished
    __: Stopped[object] = stopped
