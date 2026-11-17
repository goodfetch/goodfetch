"""The case log's entries, platform, runtime and record (ADR 0069)."""

from pathlib import Path

import pytest
from goodfetch_test_support import (
    CaseListInvalid,
    CaseLogEntryInvalid,
    ClientCaseRecord,
)
from goodfetch_test_support.case_log import (
    checked_case_log_entry,
    python_runtime,
    read_client_cases,
    rust_platform,
)
from goodfetch_test_support.generated.result_document import (
    CaseCheck,
    OracleAnswer,
    OracleOutcome,
    Platform,
    TaskId,
)

ENTRY: dict[str, object] = {
    "case": "hello-refused",
    "capability": "hello",
    "mode": "attached",
    "platform": {"os": "linux", "arch": "x86_64"},
    "runtime": "cpython-3.12",
    "checks": [{"name": "the test passed", "passed": True}],
    "evidence": {},
}


def test_an_entry_that_matches_the_bundles_schema_is_checked() -> None:
    """It comes back as the generated model."""
    entry = checked_case_log_entry(ENTRY)

    assert (entry.case, entry.capability, entry.checks) == (
        "hello-refused",
        "hello",
        [CaseCheck(name="the test passed", passed=True)],
    )


@pytest.mark.parametrize(
    "entry",
    [
        pytest.param({**ENTRY, "outcome": "passed"}, id="with-an-outcome"),
        pytest.param(
            {key: value for key, value in ENTRY.items() if key != "checks"},
            id="without-checks",
        ),
        pytest.param(
            {key: value for key, value in ENTRY.items() if key != "capability"},
            id="without-a-capability",
        ),
    ],
)
def test_an_entry_the_bundles_schema_refuses_is_invalid(entry: object) -> None:
    """An outcome is the writer's alone, and checks and capability are required."""
    with pytest.raises(CaseLogEntryInvalid):
        checked_case_log_entry(entry)


@pytest.mark.parametrize(
    ("system", "machine", "platform"),
    [
        pytest.param(
            "Linux", "x86_64", Platform(os="linux", arch="x86_64"), id="linux"
        ),
        pytest.param(
            "Linux", "aarch64", Platform(os="linux", arch="aarch64"), id="linux-arm"
        ),
        pytest.param(
            "Darwin", "arm64", Platform(os="macos", arch="aarch64"), id="macos"
        ),
        pytest.param(
            "Windows", "AMD64", Platform(os="windows", arch="x86_64"), id="windows"
        ),
        pytest.param("Windows", "x86", Platform(os="windows", arch="x86"), id="win32"),
    ],
)
def test_a_platform_is_named_as_rusts_std_env_consts_name_it(
    system: str, machine: str, platform: Platform
) -> None:
    """Every result document names the host as `std::env::consts` does."""
    assert rust_platform(system, machine) == platform


@pytest.mark.parametrize(
    ("free_threaded", "runtime"),
    [
        pytest.param(False, "cpython-3.12", id="default-build"),
        pytest.param(True, "cpython-3.12t", id="free-threaded-build"),
    ],
)
def test_a_runtime_names_the_interpreter_its_version_and_a_free_threaded_build(
    free_threaded: bool, runtime: str
) -> None:
    """A `t` marks a free-threaded build, as `python3.14t` does."""
    assert python_runtime("cpython", (3, 12), free_threaded=free_threaded) == runtime


def test_a_check_that_holds_is_recorded_as_passed() -> None:
    """The check's name and result are kept in order."""
    record = ClientCaseRecord()

    with record.check("the Client raises protocol_unsupported"):
        pass

    assert record.checks == [
        CaseCheck(name="the Client raises protocol_unsupported", passed=True)
    ]


def test_a_check_that_fails_is_recorded_as_failed_and_still_fails_the_test() -> None:
    """The failure is re-raised, so pytest still fails the test."""
    record = ClientCaseRecord()

    with (
        pytest.raises(AssertionError),
        record.check("the run ends cancelled"),
    ):
        raise AssertionError

    assert record.checks == [CaseCheck(name="the run ends cancelled", passed=False)]


def test_the_evidence_holds_what_the_test_recorded_and_leaves_out_the_rest() -> None:
    """An oracle's answer keeps only the fields it came with."""
    record = ClientCaseRecord()
    answer = OracleAnswer(outcome=OracleOutcome.verified, task=TaskId("catalog/2"))

    record.message({"method": "hello"})
    record.oracle_answer(answer)

    assert record.evidence() == {
        "messages": [{"method": "hello"}],
        "oracle_answers": [{"outcome": "verified", "task": "catalog/2"}],
    }


CASE_LIST = """
[suite]
name = "clients"
version = "1"

[[capability]]
name = "hello"

[[capability.case]]
id = "hello-refused"
covers = ["story:11-1"]

[[capability]]
name = "cancel"

[[capability.case]]
id = "cancel-mid-run"
covers = ["story:11-4"]
"""


def test_a_case_list_gives_each_cases_capability(tmp_path: Path) -> None:
    """Each case ID maps to the capability whose table holds it."""
    path = tmp_path / "cases.toml"
    path.write_text(CASE_LIST, "utf-8")

    assert read_client_cases(path) == {
        "hello-refused": "hello",
        "cancel-mid-run": "cancel",
    }


@pytest.mark.parametrize(
    "text",
    [
        pytest.param("[suite\n", id="no-toml"),
        pytest.param('[[capability]]\nname = "hello"\n', id="no-suite"),
        pytest.param(
            CASE_LIST.replace('id = "hello-refused"\n', ""), id="a-case-without-an-id"
        ),
    ],
)
def test_a_case_list_that_is_not_one_is_invalid(tmp_path: Path, text: str) -> None:
    """A list the plugin cannot read refuses the session at its start."""
    path = tmp_path / "cases.toml"
    path.write_text(text, "utf-8")

    with pytest.raises(CaseListInvalid):
        read_client_cases(path)
