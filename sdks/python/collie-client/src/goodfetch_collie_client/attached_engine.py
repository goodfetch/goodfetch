"""An Attached engine's process (ADRs 0024 and 0078).

Started from its executable with exactly the program's environment map, in a
session of its own so a terminal's Ctrl+C reaches only the host, its stdio
the Connection's transport. Its stderr is piped, never inherited: a daemon
thread passes each line to the `goodfetch.client` logger at warning and keeps
the last 64 KiB for the error its unexpected exit raises. TypeScript's
`attached-engine.ts` is its twin.
"""

from __future__ import annotations

import logging
import os
import signal
import threading
from dataclasses import dataclass
from typing import TYPE_CHECKING

import anyio
import anyio.from_thread
import anyio.lowlevel
import anyio.to_thread
from anyio.streams.stapled import StapledByteStream

from goodfetch_collie_client.generated.connection import ATTACHED_ENGINE_COMMAND
from goodfetch_collie_client.generated.errors import EngineNotFoundError

if TYPE_CHECKING:
    from collections.abc import Mapping

    from anyio.abc import ByteStream, Process

STDERR_TAIL_BYTES = 64 * 1024
"""How much of the engine's stderr its exit keeps (ADR 0078)."""

LOGGER_NAME = "goodfetch.client"
"""The logger the engine's stderr lines go to, silent unless the host listens."""


@dataclass(frozen=True)
class EngineExit:
    """How an engine's process ended."""

    exit_code: int | None
    """Its exit code; `None` when a signal ended it."""
    signal: str | None
    """The signal that ended it, such as `SIGKILL`."""
    stderr_tail: str
    """The last 64 KiB of its stderr."""


class AttachedEngine:
    """A running Attached engine."""

    def __init__(self, path: str, process: Process, stderr: _StderrReader) -> None:
        """Takes the started process and the thread reading its stderr."""
        self.path = path
        """Its executable."""
        if process.stdin is None or process.stdout is None:
            message = "invariant: the engine's stdin and stdout are piped"
            raise TypeError(message)
        self.transport: ByteStream = StapledByteStream(process.stdin, process.stdout)
        """Its stdio, the Connection's transport."""
        self._process = process
        self._stderr = stderr

    @classmethod
    async def start(
        cls, path: str, env: Mapping[str, str], not_found_fix: str
    ) -> AttachedEngine:
        """Starts the engine at `path` with exactly `env` as its environment.

        Raises:
            EngineNotFoundError: `path` is no executable file.
        """

        def not_found() -> EngineNotFoundError:
            return EngineNotFoundError(
                f"no engine executable at {path}",
                fix=not_found_fix,
                details={"path": path},
            )

        if not await _is_executable_file(path):
            raise not_found()
        read_end, write_end = os.pipe()
        try:
            process = await anyio.open_process(
                [path, ATTACHED_ENGINE_COMMAND],
                stderr=write_end,
                env=dict(env),
                start_new_session=True,
            )
        except OSError as cause:
            os.close(read_end)
            raise not_found() from cause
        finally:
            os.close(write_end)
        return cls(path, process, _StderrReader.start(read_end, anyio.Event()))

    async def exited(self) -> EngineExit:
        """Waits for the process to end and its stderr to be read."""
        code = await self._process.wait()
        await self._stderr.done.wait()
        tail = self._stderr.tail()
        if code < 0:
            return EngineExit(None, signal.Signals(-code).name, tail)
        return EngineExit(code, None, tail)

    async def stop(self, grace: float) -> EngineExit:
        """Waits up to `grace` seconds for the engine to end, then kills it.

        An Attached engine stops at the end of its input.
        """
        with anyio.move_on_after(grace):
            return await self.exited()
        self.kill()
        return await self.exited()

    def kill(self) -> None:
        """Ends the engine's process at once."""
        if self._process.returncode is None:
            try:
                self._process.kill()
            except ProcessLookupError:
                # It ended on its own meanwhile.
                return


class _StderrReader:
    """A daemon thread reading the engine's stderr until it closes.

    It tells the event loop that started it when it is done, so waiting for
    it holds no worker thread, which could keep the process alive.
    """

    def __init__(self, fd: int, done: anyio.Event) -> None:
        self._fd = fd
        self._tail = bytearray()
        self._lock = threading.Lock()
        self._token = anyio.lowlevel.current_token()
        self.done = done

    @classmethod
    def start(cls, fd: int, done: anyio.Event) -> _StderrReader:
        reader = cls(fd, done)
        thread = threading.Thread(
            target=reader._run, name="goodfetch-engine-stderr", daemon=True
        )
        thread.start()
        return reader

    def tail(self) -> str:
        with self._lock:
            data = bytes(self._tail)
        start = 0
        # A cut may split a character: its continuation bytes are 0b10xxxxxx.
        while start < len(data) and data[start] & 0xC0 == 0x80:
            start += 1
        return data[start:].decode("utf-8", errors="replace")

    def _run(self) -> None:
        logger = logging.getLogger(LOGGER_NAME)
        partial = b""
        try:
            with open(self._fd, "rb", buffering=0) as stderr:
                while chunk := stderr.read(65536):
                    with self._lock:
                        self._tail += chunk
                        del self._tail[: max(0, len(self._tail) - STDERR_TAIL_BYTES)]
                    *lines, partial = (partial + chunk).split(b"\n")
                    for line in lines:
                        logger.warning(
                            "%s",
                            line.decode("utf-8", errors="replace"),
                            extra={"source": "engine stderr"},
                        )
            if partial:
                logger.warning(
                    "%s",
                    partial.decode("utf-8", errors="replace"),
                    extra={"source": "engine stderr"},
                )
        finally:
            try:
                anyio.from_thread.run_sync(self.done.set, token=self._token)
            except RuntimeError:
                # The event loop has ended, and nobody waits any more.
                pass


async def _is_executable_file(path: str) -> bool:
    """Whether `path` names an executable file, checked off the event loop."""
    return await anyio.to_thread.run_sync(_executable_file, path)


def _executable_file(path: str) -> bool:
    return os.path.isfile(path) and os.access(path, os.X_OK)
