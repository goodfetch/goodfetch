"""J-001's `sdk-python` variant."""

import pytest


@pytest.mark.journey(id="J-001", surface="sdk-python")
def test_sdk_python() -> None:
    """Passes."""
