"""A process started where the event loop cannot start one (ADR 0081).

asyncio's selector loop, Windows's `SelectorEventLoop`, starts no
subprocess, so there the Attached engine starts with `subprocess.Popen` on
a worker thread, and a daemon thread for each of its stdin, its stdout and
its end bridges the pipe to the loop that started it: waiting on any holds
no worker thread, which could keep the process alive. Nothing here sets an
event-loop policy. Each pipe passes one chunk at a time, so a program that
stops reading stops the engine's writes, as a loop's own pipe would.
"""

from __future__ import annotations

import concurrent.futures
import functools
import subprocess
import threading
from typing import IO, TYPE_CHECKING

import anyio
import anyio.from_thread
import anyio.lowlevel
import anyio.to_thread
from anyio.abc import ByteReceiveStream, ByteSendStream, Process

if TYPE_CHECKING:
    from collections.abc import Callable, Mapping, Sequence
    from signal import Signals

    from anyio.lowlevel import EventLoopToken
    from anyio.streams.memory import (
        MemoryObjectReceiveStream,
        MemoryObjectSendStream,
    )

READ_BYTES = 65536
"""The most a read of the engine's stdout takes at once."""

_LOOP_ENDED = (RuntimeError, concurrent.futures.CancelledError)
"""What a thread's call on the loop raises once the loop has ended or is ending."""


class ThreadedProcess(Process):
    """A process `subprocess.Popen` started, its stdin and stdout bridged to a loop."""

    def __init__(self, popen: subprocess.Popen[bytes]) -> None:
        """Takes the started process and starts the threads that bridge it."""
        if popen.stdin is None or popen.stdout is None:
            message = "invariant: the process's stdin and stdout are piped"
            raise TypeError(message)
        token = anyio.lowlevel.current_token()
        self._popen = popen
        self._stdin = _PipeWriter(popen.stdin, token)
        self._stdout = _PipeReader(popen.stdout, token)
        self._ended = anyio.Event()
        _daemon("goodfetch-engine-wait", functools.partial(self._wait, token))

    @classmethod
    async def start(
        cls,
        command: Sequence[str],
        *,
        stderr: int,
        env: Mapping[str, str],
        start_new_session: bool,
    ) -> ThreadedProcess:
        """Starts `command` on a worker thread, its stdin and stdout piped.

        Raises:
            OSError: The process could not be started.
        """

        def started() -> subprocess.Popen[bytes]:
            return subprocess.Popen(
                list(command),
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=stderr,
                env=dict(env),
                start_new_session=start_new_session,
                bufsize=0,
            )

        return cls(await anyio.to_thread.run_sync(started))

    async def wait(self) -> int:
        """Waits until the process ends, and returns its exit code."""
        await self._ended.wait()
        code = self._popen.returncode
        if code is None:
            message = "invariant: an ended process has an exit code"
            raise TypeError(message)
        return code

    def terminate(self) -> None:
        """Ends the process, gracefully where the OS can."""
        self._popen.terminate()

    def kill(self) -> None:
        """Ends the process at once."""
        self._popen.kill()

    def send_signal(self, signal: Signals) -> None:
        """Sends `signal` to the process."""
        self._popen.send_signal(signal)

    @property
    def pid(self) -> int:
        """The process's ID."""
        return self._popen.pid

    @property
    def returncode(self) -> int | None:
        """Its exit code once it ended, else `None`."""
        return self._popen.returncode

    @property
    def stdin(self) -> ByteSendStream:
        """Its stdin."""
        return self._stdin

    @property
    def stdout(self) -> ByteReceiveStream:
        """Its stdout."""
        return self._stdout

    @property
    def stderr(self) -> None:
        """Never piped here: the caller hands the process its stderr."""
        return

    async def aclose(self) -> None:
        """Closes its stdin and stdout, then waits for it to end."""
        await self._stdin.aclose()
        await self._stdout.aclose()
        await self.wait()

    def _wait(self, token: EventLoopToken) -> None:
        self._popen.wait()
        _on_loop(token, self._ended.set)


class _PipeWriter(ByteSendStream):
    """A process's stdin, which a daemon thread writes each chunk sent to."""

    def __init__(self, pipe: IO[bytes], token: EventLoopToken) -> None:
        self._pipe = pipe
        self._chunks, chunks = anyio.create_memory_object_stream[bytes]()
        _daemon("goodfetch-engine-stdin", functools.partial(self._run, chunks, token))

    async def send(self, item: bytes) -> None:
        """Hands `item` to the writing thread.

        Raises:
            anyio.BrokenResourceError: The pipe broke.
            anyio.ClosedResourceError: The stream was closed.
        """
        await self._chunks.send(item)

    async def aclose(self) -> None:
        """Closes the pipe once the writing thread wrote what it was handed."""
        await self._chunks.aclose()

    def _run(
        self, chunks: MemoryObjectReceiveStream[bytes], token: EventLoopToken
    ) -> None:
        with self._pipe:
            while True:
                try:
                    chunk = anyio.from_thread.run(chunks.receive, token=token)
                except (anyio.EndOfStream, *_LOOP_ENDED):
                    return
                try:
                    view = memoryview(chunk)
                    while view:
                        view = view[self._pipe.write(view) :]
                except OSError:
                    # The process closed its stdin: the next send fails.
                    _on_loop(token, chunks.close)
                    return


class _PipeReader(ByteReceiveStream):
    """A process's stdout, which a daemon thread reads until it closes."""

    def __init__(self, pipe: IO[bytes], token: EventLoopToken) -> None:
        self._pipe = pipe
        chunks, self._chunks = anyio.create_memory_object_stream[bytes]()
        self._pending = b""
        _daemon("goodfetch-engine-stdout", functools.partial(self._run, chunks, token))

    async def receive(self, max_bytes: int = READ_BYTES) -> bytes:
        """At most `max_bytes` of what the process wrote next.

        Raises:
            anyio.EndOfStream: The process closed its stdout.
            anyio.ClosedResourceError: The stream was closed.
        """
        if not self._pending:
            self._pending = await self._chunks.receive()
        chunk, self._pending = self._pending[:max_bytes], self._pending[max_bytes:]
        return chunk

    async def aclose(self) -> None:
        """Stops passing what the process writes; the thread ends at its next read."""
        await self._chunks.aclose()

    def _run(
        self, chunks: MemoryObjectSendStream[bytes], token: EventLoopToken
    ) -> None:
        try:
            with self._pipe:
                while chunk := self._pipe.read(READ_BYTES):
                    anyio.from_thread.run(chunks.send, chunk, token=token)
        except (OSError, anyio.BrokenResourceError):
            # The pipe failed, or the stream was closed: nobody reads on.
            pass
        except _LOOP_ENDED:
            return
        _on_loop(token, chunks.close)


def _daemon(name: str, target: Callable[[], None]) -> None:
    threading.Thread(target=target, name=name, daemon=True).start()


def _on_loop(token: EventLoopToken, call: Callable[[], object]) -> None:
    """Runs `call` on the loop `token` names, unless the loop has ended."""
    try:
        anyio.from_thread.run_sync(call, token=token)
    except _LOOP_ENDED:
        # The event loop has ended, and nobody waits any more.
        pass
