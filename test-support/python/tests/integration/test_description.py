"""A description is checked against the committed schema before a test sees it."""

import json
import sys
from typing import TYPE_CHECKING

import pytest
from goodfetch_test_support import TestEnv

if TYPE_CHECKING:
    from conftest import SampleRun

SETUP_TEST = """
from goodfetch_test_support import TestEnv


def test_setup(test_env: TestEnv) -> None:
    pass
"""


def stand_in(test_env: TestEnv, description: dict[str, object]) -> str:
    """Writes an executable that prints `description` and waits for stdin to close."""
    line = json.dumps(description)
    if sys.platform == "win32":
        path = test_env.folder / "stand-in-test-env.cmd"
        path.write_text(f"@echo {line}\r\n@more > nul\r\n", "utf-8")
    else:
        path = test_env.folder / "stand-in-test-env"
        path.write_text(f"#!/bin/sh\necho '{line}'\ncat > /dev/null\n", "utf-8")
        path.chmod(0o755)
    return str(path)


@pytest.mark.parametrize(
    ("seed", "result"),
    [
        pytest.param(0, "passed", id="seed_in_range"),
        pytest.param(-1, "error", id="negative_seed"),
    ],
)
def test_test_env_refuses_a_description_that_fails_the_schema(
    sample_run: "SampleRun", test_env: TestEnv, seed: int, result: str
) -> None:
    """The fixture fails on a description the schema refuses, and passes otherwise."""
    folder = str(test_env.folder / "stand-in")
    description: dict[str, object] = {
        "folder": folder,
        "home": f"{folder}/home",
        "env": {},
        "seed": seed,
        "start_time": "2026-01-05T09:00:00Z",
        "tools": {},
    }
    tools = {"test_env": stand_in(test_env, description)}

    outcome = sample_run.run(SETUP_TEST, tools=tools)

    assert outcome.results == {"test_setup": result}, outcome.output
