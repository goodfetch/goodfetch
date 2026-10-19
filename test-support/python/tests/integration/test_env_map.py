"""The `env` fixture is the map an SDK takes as `env=` (ADRs 0069, 0090 and 0094)."""

from typing import TYPE_CHECKING

from goodfetch_test_support import TestEnv, Tool

if TYPE_CHECKING:
    from conftest import SampleRun

COLLIE_TEST = """
from collections.abc import Mapping

from goodfetch_test_support import TestEnv


def test_engine(env: Mapping[str, str], test_env: TestEnv) -> None:
    assert env["GOODFETCH_HOME"] == str(test_env.home)
    assert env["GOODFETCH_COLLIE_ENGINE_PATH"] == "stand-in/goodfetch-collie"
    assert "GOODFETCH_FOXHOUND_ENGINE_PATH" not in env
"""

FOXHOUND_TEST = """
from collections.abc import Mapping

import pytest
from goodfetch_test_support import Product, TestEnv


@pytest.fixture
def product() -> Product:
    return Product.FOXHOUND


def test_engine(env: Mapping[str, str], test_env: TestEnv) -> None:
    assert env["GOODFETCH_HOME"] == str(test_env.home)
    assert env["GOODFETCH_FOXHOUND_ENGINE_PATH"] == "stand-in/goodfetch-foxhound"
    assert "GOODFETCH_COLLIE_ENGINE_PATH" not in env
"""


def manifest(test_env: TestEnv, **entries: str) -> dict[str, str]:
    """The real `goodfetch-test-env` beside stand-in paths for the Products."""
    return {"test_env": str(test_env.tools.path(Tool.TEST_ENV)), **entries}


def test_env_map_names_collies_executable_by_default(
    sample_run: "SampleRun", test_env: TestEnv
) -> None:
    """A test runs Collie's executable unless it needs Foxhound (ADR 0094)."""
    tools = manifest(
        test_env,
        collie="stand-in/goodfetch-collie",
        foxhound="stand-in/goodfetch-foxhound",
    )

    outcome = sample_run.run(COLLIE_TEST, tools=tools)

    assert outcome.results == {"test_engine": "passed"}, outcome.output


def test_env_map_names_foxhounds_executable_for_a_test_that_needs_it(
    sample_run: "SampleRun", test_env: TestEnv
) -> None:
    """A test that overrides `product` with Foxhound gets Foxhound's variable alone."""
    tools = manifest(
        test_env,
        collie="stand-in/goodfetch-collie",
        foxhound="stand-in/goodfetch-foxhound",
    )

    outcome = sample_run.run(FOXHOUND_TEST, tools=tools)

    assert outcome.results == {"test_engine": "passed"}, outcome.output


def test_env_map_fails_when_the_manifest_lacks_the_products_executable(
    sample_run: "SampleRun", test_env: TestEnv
) -> None:
    """A manifest without Collie's executable fails the fixture, never the SDK later."""
    tools = manifest(test_env, foxhound="stand-in/goodfetch-foxhound")

    outcome = sample_run.run(COLLIE_TEST, tools=tools)

    assert outcome.results == {"test_engine": "error"}, outcome.output
