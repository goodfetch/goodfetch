"""Opens a Connection over a transport a test holds."""

from goodfetch_collie.collie import Collie


def connect_over() -> Collie[None]:
    """Connects over a transport."""
    return Collie()


__all__ = ["connect_over"]
