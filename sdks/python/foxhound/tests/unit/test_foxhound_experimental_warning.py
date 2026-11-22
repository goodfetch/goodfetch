"""A program filters ADR 0030's Experimental warning from its Product's package.

`goodfetch_foxhound` re-exports the warning's category, so a filter names
`goodfetch_foxhound.ExperimentalWarning` and never imports the Collie client
package. Collie's `test_experimental_warning.py` is its twin.
"""

import warnings

import pytest
from goodfetch_collie_client.markers import warn_experimental
from goodfetch_foxhound import ExperimentalWarning


@pytest.mark.covers("story:11-184")
def test_a_filter_on_the_re_exported_category_hides_the_experimental_warning() -> None:
    """The ignored category is the one the run-time marker warns with."""
    with warnings.catch_warnings(record=True) as heard:
        warnings.simplefilter("always")
        warnings.filterwarnings("ignore", category=ExperimentalWarning)
        warn_experimental("foxhound.filtered")

    assert heard == []
