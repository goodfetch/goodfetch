"""The `journey` result document a Journey variant run writes (ADR 0063).

It is built on the models `xtask codegen` generates from the bundle. Its
outcome is derived, never set: the rule is `goodfetch-protocol`'s
constructor's, held to the shared outcome vectors of
`test-support/fixtures/outcome-vectors/`, and every reader recomputes it
through `goodfetch-protocol`'s reader.
"""

import json
from collections.abc import Sequence
from pathlib import Path

from goodfetch_test_support.generated.result_document import (
    CheckOutcome,
    CheckResult,
    Executable,
    JourneyDocument,
    JourneyId,
    JourneyOutcome,
    OracleOutcome,
    Platform,
    ProvingGroundCopy,
    StepOutcome,
    StepResult,
    SurfacePackage,
    TaskResult,
)


def write_journey_document(
    path: Path,
    *,
    journey_id: JourneyId,
    surface: str,
    executable: Executable,
    platform: Platform,
    runtime: str,
    steps: Sequence[StepResult],
    checks: Sequence[CheckResult],
    tasks: Sequence[TaskResult],
    surface_package: SurfacePackage | None = None,
    proving_ground: ProvingGroundCopy | None = None,
    test_errors: Sequence[str] = (),
) -> JourneyOutcome:
    """Writes the document of a variant run to `path` as JSON.

    The variant is `proved` when every step passed or is `manual`, every
    check passed, every named task's oracle answered `verified` and
    `test_errors` (what failed the test outside them) is empty, and `failed`
    otherwise. No argument sets the outcome.

    Returns:
        The outcome it derived.
    """
    outcome = (
        JourneyOutcome.proved
        if all(
            step.outcome in {StepOutcome.passed, StepOutcome.manual} for step in steps
        )
        and all(check.outcome == CheckOutcome.passed for check in checks)
        and all(
            task.answer is not None and task.answer.outcome == OracleOutcome.verified
            for task in tasks
        )
        and not test_errors
        else JourneyOutcome.failed
    )
    document = JourneyDocument(
        journey_id=journey_id,
        surface=surface,
        executable=executable,
        surface_package=surface_package,
        platform=platform,
        runtime=runtime,
        proving_ground=proving_ground,
        steps=list(steps),
        checks=list(checks),
        tasks=list(tasks),
        test_errors=list(test_errors) or None,
        outcome=outcome,
    )
    # Fields left unset stay out, as Rust leaves them out; the oracle's
    # answers keep every field they came with, nulls included.
    written = {
        "kind": "journey",
        **document.model_dump(mode="json", exclude_unset=True),
    }
    for absent in ("surface_package", "proving_ground", "test_errors"):
        if written[absent] is None:
            del written[absent]
    path.write_text(json.dumps(written, indent=2) + "\n", "utf-8")
    return outcome
