"""The fixture's Product class; this module declares no `__all__`."""

from dataclasses import dataclass
from typing import Generic, TypeVar

from goodfetch_collie_client.generated.handles import Connection as ConnectionMembers

T = TypeVar("T")


@dataclass
class StartOptions:
    """A shape of data, whose fields are no members."""

    timeout: float = 0.0


class Collie(Generic[T], ConnectionMembers):
    """The Product's class."""

    @classmethod
    def connect(cls) -> "Collie[T]":
        """Connects."""
        return cls()

    def launch(self) -> None:
        """Starts a Run, and answers once it is started.

        Experimental: outside semver until it is promoted (ADR 0030).
        """

    def close(self) -> None:
        """Closes it."""

    def __enter__(self) -> "Collie[T]":
        return self

    def _flush(self) -> None:
        """Private."""


def open_over() -> None:
    """Public by its name, in a module with no `__all__`."""
