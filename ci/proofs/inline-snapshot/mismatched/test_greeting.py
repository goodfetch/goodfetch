"""A snapshot whose stored value differs: CI must fail it and write nothing."""

from inline_snapshot import snapshot


def test_greeting() -> None:
    """Compares a value with a different stored snapshot."""
    assert snapshot("goodbye") == "hello"
