"""The Scripted engine played as a Daemon (ADR 0069).

For an SDK's tests of how `connect()` finds one: `socket` publishes it in
the runtime file of a Goodfetch home, as a running Daemon is found, `tcp`
listens on loopback TCP, and `websocket` serves a WebSocket on loopback TCP,
as an Exposed Daemon `GOODFETCH_URL` names does, taking only a handshake
with the Daemon's own `Origin` and answering the subprotocol it is given
when the handshake offers it. It plays one script of the shared corpus to
one Client and exits, 0 once every step played, so a test that waits for
its exit learns whether the Client sent exactly what the script holds.
TypeScript's `scripted-daemon.ts` is its twin.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import TYPE_CHECKING, Literal, cast

from goodfetch_test_support.play_script import CORPUS
from goodfetch_test_support.tools import Tool

if TYPE_CHECKING:
    import subprocess
    from pathlib import Path

    from goodfetch_test_support.environment import TestEnv

ScriptedDaemonMode = Literal["socket", "tcp", "websocket"]
"""How a Client reaches the Scripted Daemon."""

EXIT_DEADLINE_SECONDS = 10.0
"""How long `wait` waits for the Scripted Daemon to exit once its play ended."""


class ScriptedDaemonFailed(Exception):
    """The Scripted Daemon ended, or said something else, before its address."""


@dataclass(frozen=True)
class ScriptedDaemon:
    """A Scripted engine waiting for its Client."""

    address: str
    """Where it listens: a Unix socket's path, or `host:port` on loopback TCP."""
    process: subprocess.Popen[bytes]
    """Its process, which the test's setup records and stops at teardown."""

    def wait(self) -> int:
        """Its exit code, once its play ended and it exited.

        Raises:
            subprocess.TimeoutExpired: It still runs after `EXIT_DEADLINE_SECONDS`.
        """
        return self.process.wait(EXIT_DEADLINE_SECONDS)

    def kill(self) -> int:
        """Kills it, for a test whose Client never reaches it; returns its exit code."""
        self.process.kill()
        return self.process.wait()


def start_scripted_daemon(
    test_env: TestEnv,
    script: str,
    mode: ScriptedDaemonMode,
    home: Path | None = None,
    subprotocol: str | None = None,
) -> ScriptedDaemon:
    """Starts `goodfetch-scripted-engine` playing the corpus's `script` in `mode`.

    It returns once a Client can reach it: in `socket` mode its runtime file
    is in the per-major folder of `home`, the test's own unless given, and
    `websocket` mode answers `subprotocol`, which it requires: the SDK's
    generated `WEBSOCKET_SUBPROTOCOL`, or another major's to play a Daemon
    of another protocol major. Test support takes no SDK package, so the
    test passes the SDK's own name.

    Raises:
        ToolsError: The manifest lists no Scripted engine.
        ScriptedDaemonFailed: It exited, or printed anything else, before
            saying where it listens; it is killed first.
    """
    engine = test_env.tools.path(Tool.SCRIPTED_ENGINE)
    home = test_env.home if home is None else home
    offered = [] if subprotocol is None else ["--subprotocol", subprotocol]
    process = test_env.start(
        [engine, "--script", CORPUS / script, "--mode", mode, *offered],
        env={**test_env.env, "GOODFETCH_HOME": str(home)},
    )
    line = process.stdout.readline() if process.stdout is not None else b""
    address = _address_of(line)
    if address is None:
        process.kill()
        status = process.wait()
        message = (
            f"the Scripted engine playing {script} in {mode} mode ended "
            f"(exit status {status}) without saying where it listens"
        )
        raise ScriptedDaemonFailed(message)
    return ScriptedDaemon(address, process)


def _address_of(line: bytes) -> str | None:
    """The `address` of the line the Scripted engine prints once it listens."""
    try:
        announced: object = json.loads(line)
    except ValueError:
        return None
    if not isinstance(announced, dict):
        return None
    address = cast("dict[str, object]", announced).get("address")
    return address if isinstance(address, str) else None
