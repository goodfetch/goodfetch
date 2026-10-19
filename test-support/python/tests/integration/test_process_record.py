"""No process outlives its test (ADR 0069)."""

from pathlib import Path
from typing import TYPE_CHECKING

import pytest
from goodfetch_test_support import TestEnv

if TYPE_CHECKING:
    from conftest import SampleRun

PROCESS_TEST = """
import sys

from goodfetch_test_support import TestEnv


def test_child(test_env: TestEnv) -> None:
    child = test_env.start([sys.executable, "-c", {program!r}])
    {then}
"""

FOLDER_TEST = """
from pathlib import Path

from goodfetch_test_support import TestEnv


def test_folder(test_env: TestEnv) -> None:
    Path({record!r}).write_text(str(test_env.folder), "utf-8")
"""


@pytest.mark.parametrize(
    ("program", "then", "result"),
    [
        pytest.param("pass", "child.communicate()", "passed", id="exited_child"),
        pytest.param(
            "import time; time.sleep(300)", "del child", "error", id="live_child"
        ),
    ],
)
def test_teardown_fails_a_test_whose_child_is_still_alive(
    sample_run: "SampleRun", program: str, then: str, result: str
) -> None:
    """A recorded process alive after the test fails it; one that exited does not."""
    tests = PROCESS_TEST.format(program=program, then=then)

    outcome = sample_run.run(tests)

    assert outcome.results == {"test_child": result}, outcome.output


def test_teardown_removes_the_tests_folder(
    sample_run: "SampleRun", test_env: TestEnv
) -> None:
    """Closing `goodfetch-test-env`'s stdin tears its setup down."""
    record = test_env.folder / "folder.txt"

    outcome = sample_run.run(FOLDER_TEST.format(record=str(record)))

    assert outcome.results == {"test_folder": "passed"}, outcome.output
    assert not Path(record.read_text("utf-8")).exists()
