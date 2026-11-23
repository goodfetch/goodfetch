"""A fixture adapter that calls `shout`, newer than its declared lowest version."""

from fixture_framework import shout


def welcome(name: str) -> str:
    """Welcomes `name` through the framework."""
    return shout(name)
