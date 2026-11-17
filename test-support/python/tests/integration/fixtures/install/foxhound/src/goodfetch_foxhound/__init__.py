"""A fixture Product, whose executable prints its name and version."""

from importlib.metadata import version


def main() -> None:
    """Prints `goodfetch-foxhound <version>`."""
    print(f"goodfetch-foxhound {version('goodfetch-foxhound')}")
