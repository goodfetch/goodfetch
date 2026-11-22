"""mutmut runs reach the Python programs a test starts (ADR 0072, #3199)."""

import json
import os
import subprocess
import sys

import pytest
from goodfetch_test_support import TestEnv
from goodfetch_test_support.mutation_children import (
    CHILD_FOLDER,
    HITS_VARIABLE,
    take_hits,
)

TRAMPOLINED = """\
from mutmut.mutation.trampoline import wrap_in_trampoline


def x_reach__mutmut_orig():
    return 1


def x_other__mutmut_orig():
    return 1


@wrap_in_trampoline({"_mutmut_orig": x_reach__mutmut_orig})
def reach():
    pass


@wrap_in_trampoline({"_mutmut_orig": x_other__mutmut_orig})
def other():
    pass


reach()
reach()
other()
"""
"""A program shaped as a mutated module: two functions, one reached twice."""


@pytest.mark.parametrize(
    ("mutant", "reached"),
    [
        pytest.param("stats", {"__main__.x_reach", "__main__.x_other"}, id="stats"),
        pytest.param("", set[str](), id="no_mutant"),
    ],
)
def test_a_program_writes_each_function_it_reaches_while_mutmut_counts(
    test_env: TestEnv, mutant: str, reached: set[str]
) -> None:
    """Each function once, and only while mutmut learns which tests reach each."""
    hits = test_env.folder / "hits"
    hits.mkdir()

    child = test_env.start(
        [sys.executable, "-c", TRAMPOLINED],
        env={
            **test_env.env,
            "PYTHONPATH": str(CHILD_FOLDER),
            HITS_VARIABLE: str(hits),
            "MUTANT_UNDER_TEST": mutant,
        },
    )
    child.communicate()

    assert child.returncode == 0
    assert (take_hits(hits), list(hits.iterdir())) == (reached, [])


WARNS = """\
import warnings

from mutmut.mutation.trampoline import wrap_in_trampoline


def x_deprecated__mutmut_orig():
    warnings.warn("old", DeprecationWarning, stacklevel=2)


@wrap_in_trampoline({"_mutmut_orig": x_deprecated__mutmut_orig})
def deprecated():
    pass


deprecated()
"""
"""A program shaped as a mutated module whose function warns at its caller's line."""


@pytest.mark.skipif(
    sys.version_info < (3, 12), reason="`skip_file_prefixes` came with Python 3.12"
)
def test_a_warning_from_a_mutated_function_shows_its_callers_line(
    test_env: TestEnv,
) -> None:
    """The frames of mutmut's trampoline do not count in the `stacklevel`."""
    ran = subprocess.run(
        [sys.executable, "-W", "always", "-c", WARNS],
        env={**test_env.env, "PYTHONPATH": str(CHILD_FOLDER), "MUTANT_UNDER_TEST": ""},
        capture_output=True,
        check=True,
        text=True,
        timeout=60,
    )

    assert ran.stderr.splitlines()[0] == "<string>:15: DeprecationWarning: old"


SOURCE = """\
def double(number):
    return number * 2
"""

TEST = """\
import sys

from goodfetch_test_support import TestEnv


def test_a_child_doubles(test_env: TestEnv) -> None:
    child = test_env.start(
        [sys.executable, "-c", "import sample; assert sample.double(3) == 6"]
    )
    child.communicate()
    assert child.returncode == 0
"""
"""A test that reaches `double` only through the program it starts."""

PROJECT = """\
[tool.pytest]
strict = true
addopts = ["--import-mode=importlib"]

[tool.mutmut]
source_paths = ["src"]
pytest_add_cli_args = ["-p", "no:cacheprovider", "--mutation-children"]
pytest_add_cli_args_test_selection = ["tests"]
"""


def test_mutmut_kills_each_mutant_only_a_tests_child_reaches(test_env: TestEnv) -> None:
    """The child runs the mutated copy, so its test is run against each mutant.

    `sample` is installed nowhere: the child imports it from the mutated copy.
    """
    # The test's folder is the project: `mutmut run` runs where its
    # `pyproject.toml` is, as `just mutants-sdks` runs it.
    project = test_env.folder
    (project / "src" / "sample").mkdir(parents=True)
    (project / "tests").mkdir()
    (project / "pyproject.toml").write_text(PROJECT, "utf-8")
    (project / "src" / "sample" / "__init__.py").write_text(SOURCE, "utf-8")
    (project / "tests" / "test_sample.py").write_text(TEST, "utf-8")
    # The runner's environment, which names the test tool manifest the
    # sample test's own `test_env` reads.
    environ = dict(os.environ)

    run = test_env.start([sys.executable, "-m", "mutmut", "run"], env=environ)
    output, _ = run.communicate()
    stats = test_env.start(
        [sys.executable, "-m", "mutmut", "export-cicd-stats"], env=environ
    )
    stats.communicate()

    counts = json.loads(
        (project / "mutants" / "mutmut-cicd-stats.json").read_text("utf-8")
    )
    assert (counts["total"] > 0, counts["survived"], counts["no_tests"]) == (
        True,
        0,
        0,
    ), output.decode(errors="replace")
    assert counts["killed"] == counts["total"], counts
