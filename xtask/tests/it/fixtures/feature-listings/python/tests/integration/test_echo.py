"""A test that declares a Feature with the `covers` marker."""

import pytest


@pytest.mark.covers("story:1-2")
def test_echo() -> None:
    """Passes."""
