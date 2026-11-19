"""The `journey` document's outcome follows the shared outcome vectors (ADR 0063)."""

import json
from pathlib import Path
from typing import NotRequired, TypedDict

import pytest
from goodfetch_test_support.generated.result_document import (
    CheckOutcome,
    CheckResult,
    Executable,
    JourneyId,
    JourneyOutcome,
    OracleAnswer,
    OracleOutcome,
    Platform,
    Product,
    StepOutcome,
    StepResult,
    TaskId,
    TaskResult,
)
from goodfetch_test_support.result_document import write_journey_document

VECTORS = Path(__file__).parents[3] / "fixtures" / "outcome-vectors" / "journey"


class Vector(TypedDict):
    """One shared outcome vector of the `journey` kind."""

    description: str
    steps: list[str]
    checks: list[str]
    tasks: list[str | None]
    test_errors: NotRequired[list[str]]
    outcome: str


def task(index: int, outcome: str | None) -> TaskResult:
    """The `catalog` task numbered `index`, its oracle answering `outcome`."""
    id_ = TaskId(f"catalog/{index}")
    if outcome is None:
        return TaskResult(id=id_)
    return TaskResult(
        id=id_, answer=OracleAnswer(task=id_, outcome=OracleOutcome(outcome))
    )


def write(path: Path, vector: Vector) -> JourneyOutcome:
    """Writes a variant holding what `vector` states."""
    return write_journey_document(
        path,
        journey_id=JourneyId("J-002"),
        surface="sdk-python",
        executable=Executable(product=Product.foxhound, version="0.0.0"),
        platform=Platform(os="linux", arch="x86_64"),
        runtime="cpython-3.14",
        steps=[
            StepResult(text=f"Step {index}", outcome=StepOutcome(outcome))
            for index, outcome in enumerate(vector["steps"], 1)
        ],
        checks=[
            CheckResult(text=f"Check {index}", outcome=CheckOutcome(outcome))
            for index, outcome in enumerate(vector["checks"], 1)
        ],
        tasks=[
            task(index, outcome) for index, outcome in enumerate(vector["tasks"], 1)
        ],
        test_errors=vector.get("test_errors", []),
    )


def read_vector(name: str) -> Vector:
    """The vector in `<name>.json`."""
    vector: Vector = json.loads((VECTORS / f"{name}.json").read_text("utf-8"))
    return vector


NAMES = sorted(path.stem for path in VECTORS.glob("*.json"))


@pytest.mark.parametrize("name", [pytest.param(name, id=name) for name in NAMES])
def test_write_journey_document_gives_the_outcome_of_each_vector(
    name: str, tmp_path: Path
) -> None:
    """The returned and the written outcome are the vector's."""
    vector = read_vector(name)
    path = tmp_path / "journey.json"

    outcome = write(path, vector)

    assert outcome == vector["outcome"]
    assert json.loads(path.read_text("utf-8"))["outcome"] == vector["outcome"]
