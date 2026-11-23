"""A fixture framework at 1.1.0, its newest version, which adds `shout`."""


def greet(name: str) -> str:
    """Greets `name`."""
    return f"hello, {name}"


def shout(name: str) -> str:
    """Greets `name` loudly; new in 1.1.0."""
    return f"HELLO, {name.upper()}"
