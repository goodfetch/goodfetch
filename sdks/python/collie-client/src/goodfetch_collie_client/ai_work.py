"""The result of AI work that can claim success (ADRs 0079, 0100 and 0117).

An Agent's task or a Plan replayed toward a goal: how it ended, its Verdict
and what its model claimed, read apart, with the answer inside how it ended.
A program matches `ended` on `Finished` and `Stopped`; `require_verified()`
turns a result its checks did not prove into `not_verified`, only when the
program asks. TypeScript's `ai-work.ts` is its twin.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING, Generic, Literal, TypeAlias, TypeVar

from goodfetch_collie_client.generated.errors import NotVerifiedError
from goodfetch_collie_client.goodfetch_error import runtime_message

if TYPE_CHECKING:
    from collections.abc import Mapping, Sequence

    from goodfetch_collie_client.generated.models import AxisReason, StopReason
    from goodfetch_collie_client.generated.open_values import Verdict
    from goodfetch_collie_client.json_values import JSONValue

__all__ = ["AIWorkResult", "Claim", "Ended", "Finished", "Reported", "Stopped"]

_Output = TypeVar("_Output", covariant=True)

_KnownClaim: TypeAlias = Literal["done", "gave_up"]

Claim: TypeAlias = _KnownClaim | str
"""What a model said of the work: `done` or `gave_up`; an unknown claim is kept
(ADR 0079)."""

_NOT_PROVEN = (
    "Read the result's `ended` and `verdict` to use an output its checks did not "
    "prove, or fix what the details' `reasons` name; the model's claim is never proof."
)


@dataclass(frozen=True)
class Finished(Generic[_Output]):
    """The work finished, with a complete output."""

    output: _Output
    """The answer, typed by the program's schema."""


@dataclass(frozen=True)
class Stopped(Generic[_Output]):
    """The work stopped before it finished: its reason and what it had gathered.

    Typed by the output it was working toward (ADR 0117), while its `partial`
    stays untyped JSON (ADR 0100).
    """

    reason: StopReason
    """Why it stopped."""
    partial: Mapping[str, JSONValue] | Sequence[JSONValue]
    """What it had gathered (ADR 0100): the fields of an object output, the
    elements gathered so far of a list."""


Ended: TypeAlias = Finished[_Output] | Stopped[_Output]
"""How AI work ended (ADR 0117), matched on `Finished` and `Stopped`."""


@dataclass(frozen=True)
class Reported:
    """The model's claim and its message: a signal, never proof (ADR 0100)."""

    claim: Claim
    message: str | None = None


@dataclass(frozen=True)
class AIWorkResult(Generic[_Output]):
    """AI work's result (ADR 0079)."""

    ended: Ended[_Output]
    """How the work ended, its answer inside."""
    verdict: Verdict
    """Whether the program's checks prove the work, never the model's claim."""
    verdict_reasons: Sequence[AxisReason] = ()
    """Why the Verdict is not `verified`, empty when it is."""
    reported: Reported | None = None
    """What the model claimed, `None` when no model claimed anything: the
    work stopped before a claim, or was replayed whole."""

    def require_verified(self) -> _Output:
        """The output, only when the work finished and its Verdict is `verified`.

        As httpx's `raise_for_status()`, it turns an outcome into an error
        only when asked (ADR 0079).

        Raises:
            NotVerifiedError: The work stopped or its Verdict is anything but
                `verified`; its details hold the Verdict, its reasons and the
                model's claim.

        """
        if isinstance(self.ended, Finished) and self.verdict == "verified":
            return self.ended.output
        details: dict[str, object] = {
            "verdict": self.verdict,
            "reasons": [
                reason.model_dump(mode="json", exclude_none=True)
                for reason in self.verdict_reasons
            ],
        }
        if self.reported is not None:
            reported: dict[str, str] = {"claim": self.reported.claim}
            if self.reported.message is not None:
                reported["message"] = self.reported.message
            details["reported"] = reported
        what_failed = (
            f"the AI work's Verdict is `{self.verdict}`, so its output is not verified"
        )
        raise NotVerifiedError(
            runtime_message(what_failed, fix=_NOT_PROVEN),
            details=details,
            fix=_NOT_PROVEN,
        )
