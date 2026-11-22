"""The case log the plugin writes for Client conformance cases (ADR 0069).

A sample run of a passing, a failing and a skipped case, and a test that
carries none, writes entries that match the bundle's `CaseLogEntry` schema
and set no outcome; `goodfetch-conformance write` turns them into a
`conformance` document, and `goodfetch-conformance read` accepts it.
"""

import json
from pathlib import Path
from typing import TYPE_CHECKING, TypedDict, cast

import jsonschema
import pytest
from goodfetch_test_support import TestEnv, Tool
from goodfetch_test_support.case_log import host_platform, host_runtime
from jsonschema import Draft202012Validator

if TYPE_CHECKING:
    from conftest import SampleRun

TESTS_FAILED = 1
"""pytest's exit code when a test failed."""
INTERRUPTED = 2
"""pytest's exit code when collection fails."""
USAGE_ERROR = 4
"""pytest's exit code when the session refuses to start."""

ROOT = Path(__file__).parents[4]
BUNDLE = json.loads(
    (ROOT / "crates/goodfetch-protocol/generated/bundle.json").read_text("utf-8")
)
CASE_LOG_ENTRY = {"$ref": "#/$defs/CaseLogEntry", "$defs": BUNDLE["$defs"]}

CASE_LIST = """
[suite]
name = "clients"
version = "1"

[[capability]]
name = "hello"

[[capability.case]]
id = "hello-refused"
covers = ["story:11-1"]
surfaces = ["sdk-python"]

[[capability]]
name = "cancel"

[[capability.case]]
id = "cancel-mid-run"
covers = ["story:11-4"]
surfaces = ["sdk-python"]
modes = ["attached"]

[[capability]]
name = "credits"

[[capability.case]]
id = "credits-released"
covers = ["story:11-9"]
surfaces = ["sdk-python"]
"""
"""A case list in `conformance/clients/cases.toml`'s shape: a capability whose
case passes, one whose case fails, and one whose case is skipped."""

CASES = """
import pytest
from goodfetch_test_support import ClientCaseRecord


@pytest.mark.client_case(id="hello-refused", mode="attached")
def test_refuses_a_newer_major(client_case: ClientCaseRecord) -> None:
    client_case.message({"method": "hello", "params": {"protocol": "1.4"}})
    client_case.message({"error": {"code": "protocol_unsupported"}})

    with client_case.check("the Client raises protocol_unsupported"):
        assert "protocol_unsupported" == "protocol_unsupported"


@pytest.mark.client_case(id="cancel-mid-run", mode="attached")
def test_cancels_a_run_mid_way(client_case: ClientCaseRecord) -> None:
    with client_case.check("the call sends $/cancel"):
        assert "$/cancel" == "$/cancel"
    with client_case.check("the run ends cancelled"):
        assert "completed" == "cancelled"


@pytest.mark.skip(reason="the Daemon mode comes later")
@pytest.mark.client_case(id="credits-released", mode="daemon")
def test_releases_its_credits() -> None:
    pass


def test_carries_no_case() -> None:
    pass
"""


def run_cases(
    sample_run: "SampleRun", test_env: TestEnv, tests: str = CASES
) -> tuple[int, Path, Path, str]:
    """Runs `tests` with the plugin writing the case log.

    Gives the exit code, the log, the case list and pytest's output.
    """
    case_list = test_env.folder / "cases.toml"
    case_list.write_text(CASE_LIST, "utf-8")
    log = test_env.folder / "case-log.jsonl"
    outcome = sample_run.run(
        tests, args=[f"--case-log={log}", f"--case-list={case_list}"]
    )
    return outcome.status, log, case_list, outcome.output


class Check(TypedDict):
    """One check of a case log entry."""

    name: str
    passed: bool


class Entry(TypedDict):
    """A case log entry, as far as these tests read it."""

    case: str
    checks: list[Check]


def entries(log: Path) -> list[Entry]:
    """The entries of the case log at `log`, in the order of their cases' IDs."""
    lines = log.read_text("utf-8").splitlines()
    written = [cast("Entry", json.loads(line)) for line in lines if line]
    return sorted(written, key=lambda entry: entry["case"])


def conformance(test_env: TestEnv, *args: str | Path) -> int:
    """Runs `goodfetch-conformance` with `args`, and gives its exit code."""
    process = test_env.start([test_env.tools.path(Tool.CONFORMANCE), *args])
    process.communicate()
    return process.returncode


def test_a_run_writes_one_entry_per_case_run_with_each_check_and_no_outcome(
    sample_run: "SampleRun", test_env: TestEnv
) -> None:
    """A skipped case and a test that carries none write nothing."""
    status, log, _, output = run_cases(sample_run, test_env)

    assert status == TESTS_FAILED, output
    written = entries(log)
    for entry in written:
        jsonschema.validate(entry, CASE_LOG_ENTRY, Draft202012Validator)
    platform = host_platform().model_dump(mode="json")
    assert written == [
        {
            "case": "cancel-mid-run",
            "capability": "cancel",
            "mode": "attached",
            "platform": platform,
            "runtime": host_runtime(),
            "checks": [
                {"name": "the call sends $/cancel", "passed": True},
                {"name": "the run ends cancelled", "passed": False},
                {"name": "the test passed", "passed": False},
            ],
            "evidence": {},
        },
        {
            "case": "hello-refused",
            "capability": "hello",
            "mode": "attached",
            "platform": platform,
            "runtime": host_runtime(),
            "checks": [
                {"name": "the Client raises protocol_unsupported", "passed": True},
                {"name": "the test passed", "passed": True},
            ],
            "evidence": {
                "messages": [
                    {"method": "hello", "params": {"protocol": "1.4"}},
                    {"error": {"code": "protocol_unsupported"}},
                ]
            },
        },
    ], output


def test_a_written_log_becomes_a_document_the_reader_accepts(
    sample_run: "SampleRun", test_env: TestEnv
) -> None:
    """The writer derives each case's outcome; the skipped case is not reached."""
    _, log, case_list, output = run_cases(sample_run, test_env)
    header = test_env.folder / "header.json"
    document = test_env.folder / "conformance.json"
    header.write_text(
        json.dumps(
            {
                "suite": {"name": "clients", "version": "1"},
                "target": {"name": "goodfetch-collie", "version": "0.0.0"},
                "platform": host_platform().model_dump(mode="json"),
                "runtime": host_runtime(),
                "proving_ground": {"version": "0.0.0", "seed": test_env.seed},
                "online": False,
            }
        ),
        "utf-8",
    )

    assert conformance(test_env, "write", case_list, header, log, document) == 0, output

    assert conformance(test_env, "read", document) == 0
    written = json.loads(document.read_text("utf-8"))
    assert [
        (
            capability["name"],
            capability["outcome"],
            [(case["id"], case["outcome"]) for case in capability["cases"]],
        )
        for capability in written["capabilities"]
    ] == [
        ("hello", "proved", [("hello-refused", "passed")]),
        ("cancel", "failed", [("cancel-mid-run", "failed")]),
        ("credits", "unproved", [("credits-released", "not_reached")]),
    ]


THREE_WAYS = """
import pytest
from goodfetch_test_support import ClientCaseRecord, Way, three_ways


@pytest.mark.client_case(id="hello-refused", mode="attached")
@three_ways
async def test_refuses_a_newer_major(client_case: ClientCaseRecord, way: Way) -> None:
    with client_case.check(f"the {way.value} way raises protocol_unsupported"):
        pass
"""


def test_a_three_ways_case_writes_an_entry_for_each_way(
    sample_run: "SampleRun", test_env: TestEnv
) -> None:
    """Each way is a case of its own, so each writes its entry."""
    status, log, _, output = run_cases(sample_run, test_env, THREE_WAYS)

    assert status == 0, output
    assert sorted(
        check["name"]
        for entry in entries(log)
        for check in entry["checks"]
        if check["name"] != "the test passed"
    ) == [
        "the asyncio way raises protocol_unsupported",
        "the sync way raises protocol_unsupported",
        "the trio way raises protocol_unsupported",
    ]


UNKNOWN = """
import pytest


@pytest.mark.client_case(id="hello-refusd", mode="attached")
def test_case() -> None:
    pass
"""

POSITIONAL = """
import pytest


@pytest.mark.client_case("hello-refused", mode="attached")
def test_case() -> None:
    pass
"""

NO_MODE = """
import pytest


@pytest.mark.client_case(id="hello-refused")
def test_case() -> None:
    pass
"""

LINKED = """
import pytest


@pytest.mark.client_case(id="hello-refused", mode="linked")
def test_case() -> None:
    pass
"""


@pytest.mark.parametrize(
    "tests",
    [
        pytest.param(UNKNOWN, id="unknown-case"),
        pytest.param(POSITIONAL, id="positional-id"),
        pytest.param(NO_MODE, id="no-mode"),
        pytest.param(LINKED, id="rusts-mode"),
    ],
)
def test_collection_fails_for_a_test_whose_client_case_marker_is_wrong(
    sample_run: "SampleRun", test_env: TestEnv, tests: str
) -> None:
    """A test names a case of the list and its mode, `attached` or `daemon`."""
    status, _, _, output = run_cases(sample_run, test_env, tests)

    assert status == INTERRUPTED, output


UNTAGGED = """
from goodfetch_test_support import ClientCaseRecord


def test_case(client_case: ClientCaseRecord) -> None:
    pass
"""


def test_a_test_that_records_a_case_it_does_not_carry_errors(
    sample_run: "SampleRun",
) -> None:
    """The `client_case` fixture refuses a test that names no case."""
    outcome = sample_run.run(UNTAGGED)

    assert outcome.results == {"test_case": "error"}, outcome.output
    assert "ClientCaseUntagged" in outcome.output


def test_a_case_log_without_a_case_list_refuses_the_session(
    sample_run: "SampleRun", test_env: TestEnv
) -> None:
    """The plugin cannot tell each case's capability without the list."""
    outcome = sample_run.run(
        CASES, args=[f"--case-log={test_env.folder / 'case-log.jsonl'}"]
    )

    assert outcome.status == USAGE_ERROR, outcome.output
