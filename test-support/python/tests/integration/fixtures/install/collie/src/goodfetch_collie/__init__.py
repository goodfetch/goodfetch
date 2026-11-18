"""A fixture Product, whose executable prints its name and version."""

from importlib.metadata import version


def main() -> None:
    """Prints `goodfetch-collie <version>`."""
    print(f"goodfetch-collie {version('goodfetch-collie')}")
