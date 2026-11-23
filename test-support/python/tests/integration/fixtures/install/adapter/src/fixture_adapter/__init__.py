"""A fixture adapter that calls only what its lowest framework version has."""

from fixture_framework import greet


def welcome(name: str) -> str:
    """Welcomes `name` through the framework."""
    return greet(name)
