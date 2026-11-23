"""The fixture adapter's `tests/integration`, which the lowest-version run runs."""

from fixture_adapter import welcome


def test_it_welcomes_through_the_framework() -> None:
    """The adapter's one call reaches the installed framework."""
    assert welcome("collie").lower() == "hello, collie"
