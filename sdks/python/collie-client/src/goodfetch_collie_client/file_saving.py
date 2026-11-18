"""The program's side of file saving (ADR 0116): a file is written whole or not at all.

Its bytes go into a hidden temporary file beside the path,
`.<name>.<random>.partial`, which is synced, then renamed over the path,
missing folders made first and a file there replaced. A failure, a
cancellation or a deadline removes the temporary file, so the path holds
the whole file or what it held before. A failure the path causes is told
from one of the storage by the OS's error, raised as `path_not_writable`
or `file_write_failed` with side `program`, the `OSError` chained as the
cause. Where the engine runs on the program's machine, the engine writes
the path itself under the same rule; otherwise it streams the bytes, under
credits, and this writes them. TypeScript's `file-saving.ts` is its twin.
"""

from __future__ import annotations

import base64
import errno
import math
import os
import secrets
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING, BinaryIO, NoReturn, cast

import anyio
import anyio.to_thread

from goodfetch_collie_client.framing import object_members
from goodfetch_collie_client.generated.errors import (
    FileWriteFailedError,
    PathNotWritableError,
)

if TYPE_CHECKING:
    from collections.abc import AsyncIterable, Mapping

    from goodfetch_collie_client.connection import ClientConnection
    from goodfetch_collie_client.goodfetch_error import GoodfetchError

CHUNKS_AHEAD = 16
"""How many `$/chunk` notifications the engine may send ahead of those written."""

_PATH_REASONS = frozenset(
    {
        "permission_denied",
        "read_only_filesystem",
        "is_a_directory",
        "not_a_directory",
        "invalid_filename",
        "file_too_large",
        "not_found",
    }
)
"""The reasons ADR 0116 gives `path_not_writable`; every other one is the storage's."""

_REASONS: dict[int, str] = {
    errno.ENOSPC: "storage_full",
    errno.EACCES: "permission_denied",
    errno.EPERM: "permission_denied",
    errno.EROFS: "read_only_filesystem",
    errno.EISDIR: "is_a_directory",
    errno.ENOTDIR: "not_a_directory",
    errno.ENAMETOOLONG: "invalid_filename",
    errno.EFBIG: "file_too_large",
    errno.ENOENT: "not_found",
}
"""ADR 0116's reason for each `errno` a file write meets; any other is `other`."""
if hasattr(errno, "EDQUOT"):
    _REASONS[errno.EDQUOT] = "quota_exceeded"

_WINDOWS_REASONS: dict[int, str] = {
    2: "not_found",  # ERROR_FILE_NOT_FOUND
    3: "not_found",  # ERROR_PATH_NOT_FOUND
    5: "permission_denied",  # ERROR_ACCESS_DENIED
    19: "read_only_filesystem",  # ERROR_WRITE_PROTECT
    39: "storage_full",  # ERROR_HANDLE_DISK_FULL
    112: "storage_full",  # ERROR_DISK_FULL
}
"""ADR 0116's reason for each Windows error code, read before `errno`."""

_WINDOWS_NAMES: dict[int, str] = {
    2: "ERROR_FILE_NOT_FOUND",
    3: "ERROR_PATH_NOT_FOUND",
    5: "ERROR_ACCESS_DENIED",
    19: "ERROR_WRITE_PROTECT",
    39: "ERROR_HANDLE_DISK_FULL",
    112: "ERROR_DISK_FULL",
}

_FILE_ATTRIBUTE_HIDDEN = 0x2
_FILE_ATTRIBUTE_NORMAL = 0x80


@dataclass(frozen=True)
class _Failure:
    """A failure as ADR 0116 tells it: its reason, the OS's name for it, the error."""

    reason: str
    os_code: str | None
    message: str
    cause: BaseException | None


def _failure_of(error: OSError, doing: str) -> _Failure:
    winerror = getattr(error, "winerror", None)
    if isinstance(winerror, int) and winerror in _WINDOWS_REASONS:
        reason = _WINDOWS_REASONS[winerror]
        os_code: str | None = _WINDOWS_NAMES[winerror]
    elif error.errno is not None:
        reason = _REASONS.get(error.errno, "other")
        os_code = errno.errorcode.get(error.errno)
    else:
        reason, os_code = "other", None
    return _Failure(reason, os_code, f"could not {doing}: {error}", error)


def _path_fix(call: str, reason: str) -> str:
    fixes = {
        "is_a_directory": f"Give `{call}` a file name, not a folder.",
        "not_a_directory": (
            f"Give `{call}` a path whose parent folders are folders, not files."
        ),
        "invalid_filename": f"Give `{call}` a file name this filesystem accepts.",
        "file_too_large": (
            f"Give `{call}` a path on a filesystem that holds a file this size."
        ),
    }
    return fixes.get(reason, f"Give `{call}` a path in a folder this user can write.")


def _error_of(failure: _Failure, path: str, call: str) -> GoodfetchError:
    if failure.reason in _PATH_REASONS:
        return PathNotWritableError(
            f"The path `{path}` cannot be written ({failure.message}); "
            "nothing was written there.",
            fix=_path_fix(call, failure.reason),
            details={
                "reason": failure.reason,
                "os_code": failure.os_code,
                "path": path,
            },
        )
    return FileWriteFailedError(
        f"The file could not be written at `{path}` ({failure.message}); "
        "the path holds what it held before.",
        fix=f"Free space where `{path}` is, then call `{call}` again.",
        details={
            "reason": failure.reason,
            "os_code": failure.os_code,
            "path": path,
            "side": "program",
        },
    )


def _raise(failure: _Failure, path: str, call: str) -> NoReturn:
    raise _error_of(failure, path, call) from failure.cause


def _unix_code(name: str) -> str | None:
    """The OS's name for an error found before the OS reports one, where it has one."""
    return None if sys.platform == "win32" else name


def _set_hidden(path: Path, *, hidden: bool) -> None:
    """Sets or clears the hidden attribute on Windows, where a dot hides nothing."""
    if sys.platform != "win32":
        return
    import ctypes

    attribute = _FILE_ATTRIBUTE_HIDDEN if hidden else _FILE_ATTRIBUTE_NORMAL
    if not ctypes.windll.kernel32.SetFileAttributesW(str(path), attribute):
        raise ctypes.WinError()


def _sync_folder(folder: Path) -> None:
    """Syncs `folder`, so the rename survives a crash, where a folder can be synced."""
    if sys.platform == "win32":
        return
    try:
        descriptor = os.open(folder, os.O_RDONLY)
    except OSError:
        # The file is in place; a folder that cannot be opened only weakens durability.
        return
    try:
        os.fsync(descriptor)
    except OSError:
        return
    finally:
        os.close(descriptor)


def _remove(path: Path) -> None:
    try:
        path.unlink(missing_ok=True)
    except OSError:
        # Nothing more to do: the path itself was never touched.
        return


class FileSaving:
    """A file being written to its path under ADR 0116's rule, from parts.

    `write` each part, then `finish`, or `abort` to leave the path as it was.
    """

    def __init__(
        self, given: str, target: Path, partial: Path, file: BinaryIO, call: str
    ) -> None:
        """Takes the path as given, its absolute form, the temporary file, the call."""
        self._given = given
        self._target = target
        self._partial = partial
        self._file = file
        self._call = call
        self._closed = False

    @classmethod
    async def create(cls, path: str | os.PathLike[str], *, call: str) -> FileSaving:
        """Starts the file for `path`, relative to the working directory or absolute.

        Its missing folders are made, then its hidden temporary file created beside it.

        Raises:
            PathNotWritableError: The path cannot hold a file, such as a folder
                or a path through a file.
            FileWriteFailedError: Its storage failed.
        """
        given = os.fspath(path)
        return await anyio.to_thread.run_sync(cls._create, given, call)

    @classmethod
    def _create(cls, given: str, call: str) -> FileSaving:
        target = Path(given).absolute()
        if not target.name or given.endswith(("/", os.sep)):
            _raise(
                _Failure("invalid_filename", None, f"`{given}` names no file", None),
                given,
                call,
            )
        if target.is_dir():
            _raise(
                _Failure(
                    "is_a_directory",
                    _unix_code("EISDIR"),
                    f"`{given}` is a folder",
                    None,
                ),
                given,
                call,
            )
        folder = target.parent
        try:
            folder.mkdir(parents=True, exist_ok=True)
        except FileExistsError as error:
            _raise(
                _Failure(
                    "not_a_directory",
                    _unix_code("ENOTDIR"),
                    f"a file is where the folder `{folder}` should be",
                    error,
                ),
                given,
                call,
            )
        except OSError as error:
            _raise(_failure_of(error, "make its folder"), given, call)
        partial = folder / f".{target.name}.{secrets.token_hex(6)}.partial"
        try:
            file = partial.open("xb")
        except OSError as error:
            _raise(_failure_of(error, "create the temporary file"), given, call)
        saving = cls(given, target, partial, file, call)
        try:
            _set_hidden(partial, hidden=True)
        except OSError as error:
            saving._abort()
            _raise(_failure_of(error, "hide the temporary file"), given, call)
        return saving

    async def write(self, data: bytes) -> None:
        """Writes `data` after the bytes written before.

        Raises:
            FileWriteFailedError: The storage failed, such as when it is full;
                the temporary file is removed.
            PathNotWritableError: The file grew too large for its filesystem;
                the temporary file is removed.
        """
        try:
            await anyio.to_thread.run_sync(self._write, data)
        except BaseException:
            await self.abort()
            raise

    def _write(self, data: bytes) -> None:
        try:
            self._file.write(data)
            # Flushed per part, so a full storage fails this write, not the finish.
            self._file.flush()
        except OSError as error:
            _raise(_failure_of(error, "write the file"), self._given, self._call)

    async def finish(self) -> None:
        """Syncs the file, then renames it over the path and syncs its folder.

        Raises:
            FileWriteFailedError: As `write`; the path holds what it held before.
            PathNotWritableError: As `write`.
        """
        try:
            await anyio.to_thread.run_sync(self._finish)
        except BaseException:
            await self.abort()
            raise

    def _finish(self) -> None:
        doing = "sync the file"
        try:
            self._file.flush()
            os.fsync(self._file.fileno())
            doing = "close the file"
            self._closed = True
            self._file.close()
            doing = "show the file"
            _set_hidden(self._partial, hidden=False)
            doing = "rename it over the path"
            self._partial.replace(self._target)
        except OSError as error:
            _raise(_failure_of(error, doing), self._given, self._call)
        _sync_folder(self._target.parent)

    async def abort(self) -> None:
        """Removes the temporary file, leaving the path as it was; idempotent."""
        with anyio.CancelScope(shield=True):
            await anyio.to_thread.run_sync(self._abort)

    def _abort(self) -> None:
        if not self._closed:
            self._closed = True
            try:
                self._file.close()
            except OSError:
                # Closing a file being removed: its bytes no longer matter.
                pass
        _remove(self._partial)


async def write_file_whole(
    path: str | os.PathLike[str],
    content: bytes | AsyncIterable[bytes],
    *,
    call: str,
) -> None:
    """Writes `content` at `path` whole or not at all (ADR 0116).

    A relative path resolves against the working directory, missing folders
    are made and a file there is replaced. A cancellation, such as a
    deadline's, leaves the path as it was. `call` is the call whose path it
    was, such as `save_as`, which the errors' `fix` names.

    Raises:
        PathNotWritableError: The path caused the failure.
        FileWriteFailedError: The storage did, with side `program`.
    """
    saving = await FileSaving.create(path, call=call)
    try:
        if isinstance(content, bytes):
            await saving.write(content)
        else:
            async for part in content:
                await saving.write(part)
    except BaseException:
        await saving.abort()
        raise
    await saving.finish()


def _sole(failures: BaseExceptionGroup[BaseException]) -> BaseException:
    """The one failure in `failures` once flattened, or the group if it has several."""
    leaves: list[BaseException] = []
    pending: list[BaseException] = [failures]
    while pending:
        failure = pending.pop()
        if isinstance(failure, BaseExceptionGroup):
            group = cast("BaseExceptionGroup[BaseException]", failure)
            pending.extend(group.exceptions)
        else:
            leaves.append(failure)
    return leaves[0] if len(leaves) == 1 else failures


def _chunk_of(params: object) -> bytes | None:
    """One `$/chunk` notification's bytes, or `None` when the params are no chunk."""
    members = object_members(params)
    data = None if members is None else members.get("data")
    if not isinstance(data, str):
        return None
    return base64.b64decode(data)


async def save_file(
    connection: ClientConnection,
    method: str,
    params: Mapping[str, object],
    path: str | os.PathLike[str],
    *,
    call: str,
) -> object:
    """Makes `method`, a save call such as `download.save_as`, leaving a file at `path`.

    The file lands whole or not at all (ADR 0116), whoever writes it. Where
    the engine runs on the program's machine, the call carries the path,
    made absolute against the working directory, and the engine may write
    it; otherwise, or when the engine answers it did not, the engine streams
    the bytes in `$/chunk` notifications, `CHUNKS_AHEAD` credits ahead, and
    this writes them. A failure or a cancellation leaves the path as it was.

    Returns:
        The call's result, as the engine sent it, which carries `written`.

    Raises:
        PathNotWritableError: From the engine's write or this one's.
        FileWriteFailedError: From the engine's write or this one's.
    """
    local = connection.engine.kind != "remote"
    sent_params: dict[str, object] = dict(params)
    if local:
        sent_params["path"] = str(await anyio.Path(os.fspath(path)).absolute())
    # Credits bound what the engine sends: CHUNKS_AHEAD past those written.
    send, receive = anyio.create_memory_object_stream[bytes](max_buffer_size=math.inf)
    call_id: list[int] = []
    call_sent = anyio.Event()
    saving: FileSaving | None = None

    def notified(notification: str, chunk: object) -> None:
        data = _chunk_of(chunk) if notification == "$/chunk" else None
        if data is not None:
            send.send_nowait(data)

    def sent(message_id: int) -> None:
        call_id.append(message_id)
        call_sent.set()

    async def grant(credits: int) -> None:
        await call_sent.wait()
        await connection.notify("$/credit", {"id": call_id[0], "credits": credits})

    async def write_chunks() -> None:
        nonlocal saving
        await grant(CHUNKS_AHEAD)
        async with receive:
            async for data in receive:
                saving = saving or await FileSaving.create(path, call=call)
                await saving.write(data)
                await grant(1)

    async def make_call() -> object:
        async with send:
            return await connection.stream(method, sent_params, notified, sent=sent)

    result: object = None
    try:
        failed: BaseException | None = None
        try:
            async with anyio.create_task_group() as group:
                group.start_soon(write_chunks)
                result = await make_call()
        except BaseExceptionGroup as failures:
            # One failure ends the call and the writing both: it is raised as it was,
            # its own cause kept.
            failed = _sole(failures)
        if failed is not None:
            raise failed
        members = object_members(result) or {}
        if members.get("written") is True:
            if saving is not None:
                await saving.abort()
            return result
        saving = saving or await FileSaving.create(path, call=call)
    except BaseException:
        if saving is not None:
            await saving.abort()
        raise
    await saving.finish()
    return result
