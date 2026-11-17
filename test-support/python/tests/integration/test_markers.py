"""The `journey`, `covers` and `client_case` markers, and their listing.

ADRs 0063, 0069 and 0072.
"""

import json
from typing import TYPE_CHECKING

import pytest
from goodfetch_test_support import TestEnv

if TYPE_CHECKING:
    from conftest import SampleRun

INTERRUPTED = 2
"""pytest's exit code when collection fails."""

MARKED = """
import pytest


@pytest.mark.{marker}("J-002")
def test_marked() -> None:
    pass
"""

LISTED = """
import pytest

pytestmark = pytest.mark.covers("story:3-18")


@pytest.mark.journey(id="J-002", surface="sdk-python")
@pytest.mark.covers("operation:run.start", "error:run_not_found")
def test_journey() -> None:
    pass


def test_module_level() -> None:
    pass


@pytest.mark.skipif(True, reason="the Daemon mode comes later")
@pytest.mark.client_case(id="hello-refused", mode="daemon")
@pytest.mark.client_case(id="hello-refused", mode="attached")
def test_skipped_case() -> None:
    pass
"""


@pytest.mark.parametrize(
    ("marker", "status", "results"),
    [
        pytest.param("journey", 0, {"test_marked": "passed"}, id="journey"),
        pytest.param("covers", 0, {"test_marked": "passed"}, id="covers"),
        pytest.param(
            "journee", INTERRUPTED, {"tests.test_sample": "error"}, id="unregistered"
        ),
    ],
)
def test_strict_markers_refuse_a_marker_the_plugin_does_not_register(
    sample_run: "SampleRun", marker: str, status: int, results: dict[str, str]
) -> None:
    """`journey` and `covers` are registered; a misspelled marker fails its module."""
    outcome = sample_run.run(MARKED.format(marker=marker))

    assert (outcome.status, outcome.results) == (status, results), outcome.output


def test_marker_listing_names_each_test_with_its_markers_values(
    sample_run: "SampleRun", test_env: TestEnv
) -> None:
    """The listing the Feature scanner, the Journey check and the clients check read.

    It gives each test's `journey`, `covers` and `client_case` values, and
    whether a `skip` or `skipif` marker may keep it from running (ADRs 0069
    and 0072).
    """
    listing = test_env.folder / "markers.json"

    outcome = sample_run.run(
        LISTED, args=["--collect-only", f"--marker-listing={listing}"]
    )

    assert outcome.status == 0, outcome.output
    assert json.loads(listing.read_text("utf-8")) == {
        "tests": [
            {
                "id": "tests/test_sample.py::test_journey",
                "journey": [{"id": "J-002", "surface": "sdk-python"}],
                "covers": ["operation:run.start", "error:run_not_found", "story:3-18"],
                "client_case": [],
                "skipped": False,
            },
            {
                "id": "tests/test_sample.py::test_module_level",
                "journey": [],
                "covers": ["story:3-18"],
                "client_case": [],
                "skipped": False,
            },
            {
                "id": "tests/test_sample.py::test_skipped_case",
                "journey": [],
                "covers": ["story:3-18"],
                "client_case": [
                    {"id": "hello-refused", "mode": "attached"},
                    {"id": "hello-refused", "mode": "daemon"},
                ],
                "skipped": True,
            },
        ]
    }


SELECTED = """
import pytest


@pytest.mark.journey(id="J-002", surface="sdk-python")
def test_sdk_python() -> None:
    pass


@pytest.mark.journey(id="J-003", surface="sdk-python")
def test_other_journey() -> None:
    pass


def test_unmarked() -> None:
    pass
"""


def test_journey_filter_selects_only_its_variants_test(sample_run: "SampleRun") -> None:
    """The `-m` filter `xtask journeys select` prints for J-002@sdk-python."""
    outcome = sample_run.run(
        SELECTED, args=["-m", 'journey(id="J-002", surface="sdk-python")']
    )

    assert (outcome.status, outcome.results) == (
        0,
        {"test_sdk_python": "passed"},
    ), outcome.output
