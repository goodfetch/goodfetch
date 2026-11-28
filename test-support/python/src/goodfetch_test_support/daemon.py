"""The Daemon `goodfetch-test-env --daemon` starts in the test's home (ADRs 0024, 0069).

It is run by the commit's executable of the test's Product, Collie's unless
the test needs Foxhound (ADR 0094), and holds what a Client needs to reach
it: its socket and token. It stops, leaving no runtime file, when the setup
is torn down. Python reads what the Rust setup started, and starts nothing.
"""

from dataclasses import dataclass
from typing import Self, cast


class DaemonNotRequested(Exception):
    """A test took `daemon` without overriding `with_daemon` to `True`."""


@dataclass(frozen=True)
class Daemon:
    """A started Daemon, as `goodfetch-test-env` describes it."""

    socket: str
    """Where it listens: its Unix socket's path on macOS and Linux, its
    loopback TCP address (`127.0.0.1:<port>`) on Windows."""
    token: str
    """The per-launch token a Client's `hello` carries."""
    http: str | None = None
    """The HTTP JSON-RPC listener's base URL (`http://127.0.0.1:<port>`),
    present only with `with_http_listener`."""
    http_token: str | None = None
    """The Bearer token the HTTP listener takes in `Authorization: Bearer …`,
    present only with `with_http_listener`."""

    @classmethod
    def from_json(cls, value: object) -> Self:
        """Reads the description's `daemon`, already checked by its schema."""
        fields = cast("dict[str, str]", value)
        return cls(
            socket=fields["socket"],
            token=fields["token"],
            http=fields.get("http"),
            http_token=fields.get("http_token"),
        )
