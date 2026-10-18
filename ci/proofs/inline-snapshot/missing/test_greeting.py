"""A snapshot with no stored value: CI must fail it and write nothing."""

from inline_snapshot import snapshot


def test_greeting() -> None:
    """Compares a value with an empty snapshot."""
    assert snapshot() == "hello"
