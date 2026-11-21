"""AI work that can claim success (ADRs 0079 and 0117).

How it ended, its Verdict and its model's claim are read apart, and
`require_verified()` hands over the output only when the work finished and
its checks prove it, raising `not_verified` otherwise. TypeScript's
`ai-work.test.ts` is its twin.
"""

from __future__ import annotations

from dataclasses import dataclass, replace

import pytest
from goodfetch_collie_client.ai_work import AIWorkResult, Finished, Reported, Stopped
from goodfetch_collie_client.generated.errors import NotVerifiedError
from goodfetch_collie_client.generated.models import AxisReason, StopReason


@dataclass(frozen=True)
class Author:
    """An output a program's schema types."""

    name: str
    born: int


ADA = Author(name="Ada", born=1815)
FINISHED: AIWorkResult[Author] = AIWorkResult(
    ended=Finished(ADA),
    verdict="verified",
    reported=Reported(claim="done", message="found her"),
)
STOPPED: Stopped[Author] = Stopped(
    reason=StopReason.model_validate({"kind": "timeout"}), partial={"name": "Ada"}
)


def _reason(code: str, message: str, call_id: str | None = None) -> AxisReason:
    return AxisReason(code=code, message=message, call_id=call_id)


@pytest.mark.covers("story:11-34")
def test_a_verified_finished_result_hands_over_its_output() -> None:
    """A finished result whose Verdict is `verified` returns its output."""
    assert FINISHED.require_verified() == ADA


@pytest.mark.covers("story:11-34", "error:not_verified")
def test_a_finished_result_its_checks_did_not_prove_raises_not_verified() -> None:
    """Its details hold the Verdict, the reasons in wire names and the model's claim."""
    result = replace(
        FINISHED,
        verdict="rejected",
        verdict_reasons=[
            _reason("success_check_failed", "the heading was not shown", "c1")
        ],
    )

    with pytest.raises(NotVerifiedError) as raised:
        result.require_verified()

    error = raised.value
    assert (error.code, error.group, error.retryable, error.fix is not None) == (
        "not_verified",
        "page",
        False,
        True,
    )
    assert error.details == {
        "verdict": "rejected",
        "reasons": [
            {
                "code": "success_check_failed",
                "message": "the heading was not shown",
                "call_id": "c1",
            }
        ],
        "reported": {"claim": "done", "message": "found her"},
    }
    assert "rejected" in str(error)


@pytest.mark.covers("story:11-34", "error:not_verified")
def test_a_stopped_result_raises_not_verified_even_with_a_verified_verdict() -> None:
    """Without a claim, the details hold the Verdict and its reasons only."""
    with pytest.raises(NotVerifiedError) as raised:
        replace(FINISHED, ended=STOPPED, reported=None).require_verified()

    assert raised.value.details == {"verdict": "verified", "reasons": []}


@pytest.mark.covers("story:11-34", "error:not_verified")
def test_an_unknown_verdict_is_never_verified() -> None:
    """A Verdict a newer engine adds is kept, and refused."""
    with pytest.raises(NotVerifiedError) as raised:
        replace(FINISHED, verdict="proven").require_verified()

    assert raised.value.details is not None
    assert raised.value.details["verdict"] == "proven"


@pytest.mark.covers("story:11-34")
def test_how_it_ended_its_verdict_and_the_models_claim_are_read_apart() -> None:
    """A stopped task's partial output sits beside its Verdict and the model's claim."""
    result = replace(
        FINISHED,
        ended=STOPPED,
        verdict="needs_review",
        verdict_reasons=[_reason("stopped", "the task stopped before it finished")],
        reported=Reported(claim="done"),
    )

    match result.ended:
        case Finished(output=output):
            name: object = output.name
        case Stopped(partial=partial):
            name = partial["name"] if isinstance(partial, dict) else None

    assert (name, result.verdict, result.reported) == (
        "Ada",
        "needs_review",
        Reported(claim="done"),
    )
