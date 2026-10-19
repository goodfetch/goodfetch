"""Tests run in random order and in parallel without sharing a folder (ADR 0062)."""

import json
from typing import TYPE_CHECKING

from goodfetch_test_support import TestEnv

if TYPE_CHECKING:
    from conftest import SampleRun

RECORDING = """
import json
from pathlib import Path

from goodfetch_test_support import TestEnv


def record(test_env: TestEnv, name: str) -> None:
    setup = {{"folder": str(test_env.folder), "home": str(test_env.home)}}
    Path({records!r}, name + ".json").write_text(json.dumps(setup), "utf-8")


def test_first(test_env: TestEnv) -> None:
    record(test_env, "first")


def test_second(test_env: TestEnv) -> None:
    record(test_env, "second")
"""


def test_parallel_shuffled_tests_never_share_a_folder_or_a_home(
    sample_run: "SampleRun", test_env: TestEnv
) -> None:
    """Under pytest-xdist and pytest-randomly each test gets its own setup."""
    records = test_env.folder / "records"
    records.mkdir()

    outcome = sample_run.run(
        RECORDING.format(records=str(records)),
        args=["-n", "2", "-p", "randomly", "--randomly-seed=17"],
    )

    assert outcome.results == {"test_first": "passed", "test_second": "passed"}, (
        outcome.output
    )
    first = json.loads((records / "first.json").read_text("utf-8"))
    second = json.loads((records / "second.json").read_text("utf-8"))
    assert first["folder"] != second["folder"]
    assert first["home"] != second["home"]
