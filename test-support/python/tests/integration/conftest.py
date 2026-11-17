"""Runs pytest on sample tests in the test's folder, the plugin loaded as in any run."""

import json
import os
import re
import sys
import xml.etree.ElementTree as ElementTree
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path

import pytest
from goodfetch_test_support import TestEnv

CONFIGURATION = """\
[tool.pytest]
strict = true
strict_markers = true
addopts = ["--import-mode=importlib"]
"""
"""The settings of the workspace's `[tool.pytest]` that a sample run keeps."""


@dataclass(frozen=True)
class Outcome:
    """What one pytest run reported."""

    status: int
    """pytest's exit code."""
    results: Mapping[str, str]
    """Each reported test's name: `passed`, `failure`, `error` or `skipped`.

    Empty when the session was refused before it started.
    """
    project: Path
    """The folder the sample tests ran in."""
    output: str
    """pytest's own output, shown when an assertion on the run fails."""


@dataclass(frozen=True)
class SampleRun:
    """Runs pytest on sample test files in the test's folder."""

    test_env: TestEnv

    def run(
        self,
        tests: str,
        *,
        tools: Mapping[str, str] | None = None,
        args: Sequence[str] = (),
        env: Mapping[str, str] | None = None,
        interpreter: Sequence[str] = (),
    ) -> Outcome:
        """Writes `tests` as a test module and runs pytest on it.

        Args:
            tests: The test module's source.
            tools: The test tool manifest's entries the run reads; the real
                manifest's unless given.
            args: More pytest arguments.
            env: More environment variables, over this run's own.
            interpreter: Options for the Python interpreter, before `-m pytest`.
        """
        project = self.test_env.folder / "project"
        (project / "tests").mkdir(parents=True)
        (project / "pyproject.toml").write_text(CONFIGURATION, "utf-8")
        (project / "tests" / "test_sample.py").write_text(tests, "utf-8")
        environ = dict(os.environ)
        if tools is not None:
            manifest = self.test_env.folder / "test-tools.json"
            manifest.write_text(json.dumps(tools), "utf-8")
            environ["GOODFETCH_TEST_TOOLS"] = str(manifest)
        environ.update(env or {})
        junit = project / "junit.xml"
        process = self.test_env.start(
            [
                sys.executable,
                *interpreter,
                "-m",
                "pytest",
                "-p",
                "no:cacheprovider",
                f"--rootdir={project}",
                f"--junitxml={junit}",
                *args,
                str(project / "tests"),
            ],
            env=environ,
        )
        output, _ = process.communicate()
        return Outcome(
            process.returncode,
            _results(junit),
            project,
            output.decode(errors="replace"),
        )


def _results(junit: Path) -> dict[str, str]:
    results: dict[str, str] = {}
    # A session refused before it starts writes no JUnit file.
    if not junit.exists():
        return results
    for case in ElementTree.parse(junit).iter("testcase"):
        kinds = [
            child.tag for child in case if child.tag in {"failure", "error", "skipped"}
        ]
        results[case.attrib["name"]] = kinds[0] if kinds else "passed"
    return results


@pytest.fixture
def sample_run(test_env: TestEnv) -> SampleRun:
    """Runs pytest on sample tests written into the test's folder."""
    return SampleRun(test_env)


def _catalog_product(page: str) -> dict[str, str]:
    def field(pattern: str) -> str:
        found = re.search(pattern, page)
        assert found is not None, pattern
        return found[1]

    return {
        "id": field(r'<article class="product" data-id="([^"]+)"'),
        "name": field(r'<h1 class="name">([^<]+)</h1>'),
        "price": field(r'data-price="([^"]+)"'),
    }


@pytest.fixture
def catalog_product() -> Callable[[str], dict[str, str]]:
    """Reads the product a `catalog` product page shows, as any tool would."""
    return _catalog_product
