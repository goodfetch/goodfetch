"""A test that requests `monkeypatch` or `mocker` fails at collection (ADR 0062)."""

from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from conftest import SampleRun

INTERRUPTED = 2
"""pytest's exit code when collection fails."""

DIRECT = """
import pytest


def test_patched(monkeypatch: pytest.MonkeyPatch) -> None:
    pass
"""

THROUGH_A_FIXTURE = """
import pytest


@pytest.fixture
def patched(monkeypatch: pytest.MonkeyPatch) -> None:
    pass


def test_patched(patched: None) -> None:
    pass
"""

MOCKER = """
import pytest


@pytest.fixture
def mocker() -> None:
    pass


def test_patched(mocker: None) -> None:
    pass
"""

CLEAN = """
import pytest


@pytest.fixture
def value() -> int:
    return 1


def test_patched(value: int) -> None:
    assert value == 1
"""


@pytest.mark.parametrize(
    "tests",
    [
        pytest.param(DIRECT, id="monkeypatch_requested"),
        pytest.param(THROUGH_A_FIXTURE, id="monkeypatch_through_a_fixture"),
        pytest.param(MOCKER, id="mocker_requested"),
    ],
)
def test_collection_fails_for_a_test_reaching_a_patching_fixture(
    sample_run: "SampleRun", tests: str
) -> None:
    """The plugin walks each test's fixture closure and refuses the test."""
    outcome = sample_run.run(tests)

    assert (outcome.status, outcome.results) == (
        INTERRUPTED,
        {"test_patched": "error"},
    ), outcome.output


def test_collection_passes_for_a_test_reaching_no_patching_fixture(
    sample_run: "SampleRun",
) -> None:
    """A test whose fixtures never request `monkeypatch` or `mocker` runs."""
    outcome = sample_run.run(CLEAN)

    assert (outcome.status, outcome.results) == (
        0,
        {"test_patched": "passed"},
    ), outcome.output
