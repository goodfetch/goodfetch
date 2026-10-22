"""No process outlives its test (ADR 0069)."""

import json
import shutil
import sys
import time
from pathlib import Path
from typing import TYPE_CHECKING

import pytest
from goodfetch_test_support import ProcessesAlive, TestEnv

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


STAYS_ALIVE = """\
import json
import sys
import time

print(json.dumps({description}), flush=True)
time.sleep(300)
"""
"""A `goodfetch-test-env` stand-in that describes a setup and never ends."""


def test_teardown_fails_when_goodfetch_test_env_stays_alive(test_env: TestEnv) -> None:
    """A live `goodfetch-test-env` fails teardown at the deadline the test sets."""
    description = {
        "folder": "/nonexistent/fake test é",
        "home": "/nonexistent/fake test é/home",
        "env": {"GOODFETCH_HOME": "/nonexistent/fake test é/home"},
        "seed": 1,
        "start_time": "2026-01-05T09:00:00Z",
        "tools": {},
    }
    program = test_env.folder / "stays-alive"
    program.write_text(
        f"#!{sys.executable}\n"
        + STAYS_ALIVE.format(description=json.dumps(description)),
        "utf-8",
    )
    program.chmod(0o755)
    manifest = test_env.folder / "test-tools.json"
    manifest.write_text(json.dumps({"test_env": str(program)}), "utf-8")
    setup = TestEnv.open(
        "stays alive",
        {"GOODFETCH_TEST_TOOLS": str(manifest)},
        teardown_deadline=0.2,
    )
    began = time.monotonic()

    with pytest.raises(ProcessesAlive):
        setup.close()

    assert time.monotonic() - began < 5


FAILING_FOLDER_TEST = """
from pathlib import Path

from goodfetch_test_support import TestEnv


def test_folder(test_env: TestEnv) -> None:
    Path({record!r}).write_text(str(test_env.folder), "utf-8")
    raise AssertionError("the sample test fails on purpose")
"""


def test_teardown_keeps_a_failed_tests_folder(
    sample_run: "SampleRun", test_env: TestEnv
) -> None:
    """A failed test's folder stays for its evidence, as a Rust test's does."""
    record = test_env.folder / "folder.txt"

    outcome = sample_run.run(FAILING_FOLDER_TEST.format(record=str(record)))

    assert outcome.results == {"test_folder": "failure"}, outcome.output
    folder = Path(record.read_text("utf-8"))
    assert folder.is_dir()
    shutil.rmtree(folder)
