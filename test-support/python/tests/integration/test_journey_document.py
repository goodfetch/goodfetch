"""The `journey` result documents the helper writes (ADR 0063).

Each matches the bundle's schema and `goodfetch-protocol`'s reader accepts
it, through `goodfetch-conformance read` from the test tool manifest. A
sample Journey-shaped test drives the Proving ground's `catalog/2` task and
its oracle, and writes one `proved` and one `failed` document.
"""

import http.client
import json
from collections.abc import Callable, Generator
from contextlib import contextmanager
from pathlib import Path
from typing import Any, TypedDict, cast
from urllib.parse import urlsplit

import jsonschema
import pytest
from goodfetch_test_support import TestEnv, Tool, proving_ground_routes
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
    ProvingGroundCopy,
    StepOutcome,
    StepResult,
    TaskId,
    TaskResult,
)
from goodfetch_test_support.result_document import write_journey_document
from goodfetch_test_support.schemas import validate
from jsonschema import Draft202012Validator
from jsonschema.exceptions import ValidationError

pytestmark = pytest.mark.covers("story:2-84")

ROOT = Path(__file__).parents[4]
BUNDLE = json.loads(
    (ROOT / "crates/goodfetch-protocol/generated/bundle.json").read_text("utf-8")
)
RESULT_DOCUMENT = {"$ref": "#/$defs/ResultDocument", "$defs": BUNDLE["$defs"]}
VECTORS = ROOT / "test-support/fixtures/outcome-vectors/journey"
NAMES = sorted(path.stem for path in VECTORS.glob("*.json"))


class Vector(TypedDict):
    """One shared outcome vector of the `journey` kind."""

    steps: list[str]
    checks: list[str]
    tasks: list[str | None]


def schema_errors(path: Path) -> list[str]:
    """What the bundle's schema finds wrong in the document at `path`."""
    document = json.loads(path.read_text("utf-8"))
    try:
        jsonschema.validate(document, RESULT_DOCUMENT, Draft202012Validator)
    except ValidationError as error:
        return [f"{error.json_path}: {error.message}"]
    return []


def read(test_env: TestEnv, path: Path) -> int:
    """Runs `goodfetch-conformance read <path>` and gives its exit code."""
    reader = test_env.start([test_env.tools.path(Tool.CONFORMANCE), "read", path])
    reader.communicate()
    return reader.returncode


def task(index: int, outcome: str | None) -> TaskResult:
    """The `catalog` task numbered `index`, its oracle answering `outcome`."""
    id_ = TaskId(f"catalog/{index}")
    if outcome is None:
        return TaskResult(id=id_)
    return TaskResult(
        id=id_, answer=OracleAnswer(task=id_, outcome=OracleOutcome(outcome))
    )


@pytest.mark.parametrize("name", [pytest.param(name, id=name) for name in NAMES])
def test_write_journey_document_writes_each_vector_as_a_document_the_reader_accepts(
    name: str, test_env: TestEnv
) -> None:
    """The document matches the schema and the reader recomputes its outcome."""
    vector: Vector = json.loads((VECTORS / f"{name}.json").read_text("utf-8"))
    path = test_env.folder / "journey.json"
    write_journey_document(
        path,
        journey_id=JourneyId("J-002"),
        surface="sdk-python",
        executable=Executable(product=Product.collie, version="0.0.0"),
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
    )

    code = read(test_env, path)

    assert schema_errors(path) == []
    assert code == 0


class ProvingGround:
    """A running local copy of the Proving ground, reached at its address."""

    def __init__(self, address: str) -> None:
        """Reaches the copy `serve` reported at `address` (`host:port`)."""
        self.host, port = address.rsplit(":", 1)
        self.port = int(port)

    def exchange(
        self, method: str, host: str, path: str, body: object | None = None
    ) -> str:
        """One HTTP exchange with the copy, naming `host` in `Host`."""
        connection = http.client.HTTPConnection(self.host, self.port, timeout=30)
        try:
            connection.request(
                method,
                path,
                None if body is None else json.dumps(body),
                {"Host": host, "Content-Type": "application/json"},
            )
            return connection.getresponse().read().decode("utf-8")
        finally:
            connection.close()


@contextmanager
def proving_ground(test_env: TestEnv) -> Generator[ProvingGround]:
    """Serves the local copy on the test's seed until the block ends."""
    serve = test_env.start(
        [
            test_env.tools.path(Tool.PROVING_GROUND),
            *("serve", "--seed", str(test_env.seed), "--copy", "local"),
            *("--address", "127.0.0.1:0", "--network-profile", "none"),
        ]
    )
    try:
        assert serve.stdout is not None
        reported: object = json.loads(serve.stdout.readline())
        validate(reported, "proving-ground-serve-line.schema.json")
        yield ProvingGround(cast("dict[str, str]", reported)["address"])
    finally:
        serve.kill()
        serve.communicate()


def extract_the_product(
    test_env: TestEnv, product: Callable[[str], dict[str, str]], *, wrong_price: bool
) -> JourneyOutcome:
    """A Journey-shaped run of `catalog/2`, which writes its document.

    It reads the product page, submits its Record to the oracle (with a
    wrong price when `wrong_price`), and orders the product only when the
    oracle verified it. Each step's outcome is recorded as it happens.
    """
    with proving_ground(test_env) as copy:
        routes = proving_ground_routes()
        listing: dict[str, Any] = json.loads(
            copy.exchange("GET", "tasks", routes.tasks)
        )
        entry = next(entry for entry in listing["tasks"] if entry["id"] == "catalog/2")
        url = urlsplit(cast("str", entry["start_url"]))
        record = product(copy.exchange("GET", url.netloc, url.path))
        submitted = {**record, "price": "0.01"} if wrong_price else record
        submission = {"task": "catalog/2", "status": "success", "records": [submitted]}
        validate(submission, "proving-ground-oracle-submission.schema.json")
        answer = cast(
            "dict[str, Any]",
            json.loads(
                copy.exchange(
                    "POST",
                    "oracle",
                    routes.oracle,
                    submission,
                )
            ),
        )
    verified = answer["outcome"] == "verified"
    task_id = TaskId("catalog/2")
    return write_journey_document(
        test_env.folder / "journey.json",
        journey_id=JourneyId("J-002"),
        surface="sdk-python",
        executable=Executable(product=Product.collie, version="0.0.0"),
        platform=Platform(os="linux", arch="x86_64"),
        runtime="cpython-3.14",
        proving_ground=ProvingGroundCopy.model_validate(listing["proving_ground"]),
        steps=[
            StepResult(text="Read the product page", outcome=StepOutcome.passed),
            StepResult(
                text="Submit the product as the task's Record",
                outcome=StepOutcome.passed,
            ),
            StepResult(
                text="Order the product at its price",
                outcome=StepOutcome.passed if verified else StepOutcome.not_reached,
            ),
        ],
        checks=[
            CheckResult(
                text="The oracle verifies the Record",
                task=task_id,
                outcome=CheckOutcome.passed if verified else CheckOutcome.failed,
            )
        ],
        tasks=[TaskResult(id=task_id, answer=OracleAnswer.model_validate(answer))],
    )


def test_a_journey_shaped_test_writes_a_proved_document_when_the_oracle_verifies(
    test_env: TestEnv, catalog_product: Callable[[str], dict[str, str]]
) -> None:
    """Every step and check passed and the task is `verified`."""
    outcome = extract_the_product(test_env, catalog_product, wrong_price=False)

    path = test_env.folder / "journey.json"
    assert outcome == JourneyOutcome.proved
    assert schema_errors(path) == []
    assert read(test_env, path) == 0


def test_a_journey_shaped_test_writes_a_failed_document_beside_a_false_success(
    test_env: TestEnv, catalog_product: Callable[[str], dict[str, str]]
) -> None:
    """A step not reached and a task answering `false_success` fail the variant."""
    outcome = extract_the_product(test_env, catalog_product, wrong_price=True)

    path = test_env.folder / "journey.json"
    document = json.loads(path.read_text("utf-8"))
    assert outcome == JourneyOutcome.failed
    assert document["outcome"] == "failed"
    assert [step["outcome"] for step in document["steps"]] == [
        "passed",
        "passed",
        "not_reached",
    ]
    assert document["tasks"][0]["answer"]["outcome"] == "false_success"
    assert read(test_env, path) == 0
