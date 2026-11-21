"""A listener's handle."""


class Closable:
    """A base the SDK declares, whose members its subclasses export."""

    def close(self) -> None:
        """Closes it."""


class Subscription(Closable):
    """What removes a listener."""
