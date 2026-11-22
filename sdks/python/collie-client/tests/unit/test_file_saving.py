"""The program's side of file saving (ADR 0116) against the real filesystem.

A file lands whole or not at all, and a failure is `path_not_writable` or
`file_write_failed` with side `program`, its `OSError` chained as the cause.
The save helper over a Connection whose engine the test plays: the engine
writes the path, or streams the bytes for the SDK to write. TypeScript's
`file-saving.test.ts` is its twin.
"""

from __future__ import annotations

import base64
import contextlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import TYPE_CHECKING

import anyio
import pytest
from anyio.abc import ByteStream
from goodfetch_collie_client._file_saving import (
    FileSaving,
    save_file,
    write_file_whole,
)
from goodfetch_collie_client.connection import ClientConnection
from goodfetch_collie_client.generated.errors import (
    ERROR_CLASSES,
    FileWriteFailedError,
    PathNotWritableError,
)
from goodfetch_collie_client.generated.models import Product

if TYPE_CHECKING:
    from collections.abc import AsyncIterator, Generator

pytestmark = pytest.mark.anyio

RUNS_AS_ROOT = hasattr(os, "getuid") and os.getuid() == 0

TEST_UNSHARE = "/usr/local/libexec/goodfetch-test-tools/unshare"
"""The test-only `unshare` that `ci/os-services.sh setup` installs."""


@pytest.fixture(params=["asyncio", "trio"])
def anyio_backend(request: pytest.FixtureRequest) -> str:
    """Each test runs on asyncio and on Trio (ADR 0081)."""
    return str(request.param)


async def failure(awaitable: object) -> BaseException:
    """The error `awaitable` raises."""
    try:
        await awaitable  # type: ignore[misc]
    except Exception as error:
        return error
    message = "the write succeeded"
    raise AssertionError(message)


def names_in(folder: Path) -> list[str]:
    """The names in `folder`, the temporary files among them."""
    return sorted(entry.name for entry in folder.iterdir())


async def csv_parts() -> AsyncIterator[bytes]:
    """A small CSV file in two parts."""
    yield b"a,b\n"
    yield b"1,2\n"


async def test_makes_the_missing_folders_and_leaves_only_the_file(
    tmp_path: Path,
) -> None:
    """Missing parent folders are made, and no temporary file is left."""
    path = tmp_path / "a" / "b" / "report.pdf"

    await write_file_whole(path, b"%PDF", call="save_as")

    assert path.read_bytes() == b"%PDF"
    assert names_in(path.parent) == ["report.pdf"]


async def test_resolves_a_relative_path_against_the_working_directory(
    tmp_path: Path,
) -> None:
    """A relative path lands under the working directory."""
    before = Path.cwd()
    os.chdir(tmp_path)
    try:
        await write_file_whole(Path("out") / "page.html", b"<html>", call="save_as")
    finally:
        os.chdir(before)

    assert (tmp_path / "out" / "page.html").read_bytes() == b"<html>"


async def test_replaces_a_file_at_the_path_whole_from_parts(tmp_path: Path) -> None:
    """A repeat write replaces the earlier file whole."""
    path = tmp_path / "data.csv"
    path.write_text("what it held, and longer than the new file")

    await write_file_whole(path, csv_parts(), call="save_as")

    assert path.read_bytes() == b"a,b\n1,2\n"
    assert names_in(tmp_path) == ["data.csv"]


@pytest.mark.skipif(
    RUNS_AS_ROOT or sys.platform == "win32", reason="permissions bind a user, not root"
)
async def test_raises_path_not_writable_in_a_folder_the_user_cannot_write(
    tmp_path: Path,
) -> None:
    """An unwritable folder: `path_not_writable`, the `OSError` its cause."""
    locked = tmp_path / "locked"
    locked.mkdir()
    locked.chmod(0o500)
    path = locked / "report.pdf"
    try:
        error = await failure(write_file_whole(path, b"%PDF", call="save_as"))
    finally:
        locked.chmod(0o700)

    assert isinstance(error, PathNotWritableError)
    assert (error.code, error.retryable) == ("path_not_writable", False)
    assert error.fix == "Give `save_as` a path in a folder this user can write."
    assert error.details == {
        "reason": "permission_denied",
        "os_code": "EACCES",
        "path": str(path),
    }
    assert isinstance(error.__cause__, PermissionError)
    assert names_in(locked) == []


async def test_raises_path_not_writable_naming_a_folder_at_the_path(
    tmp_path: Path,
) -> None:
    """A folder at the path: `path_not_writable`, `is_a_directory`."""
    path = tmp_path / "taken"
    path.mkdir()

    error = await failure(write_file_whole(path, b"x", call="save_as"))

    assert isinstance(error, PathNotWritableError)
    assert error.fix == "Give `save_as` a file name, not a folder."
    assert error.details["reason"] == "is_a_directory"


async def test_raises_path_not_writable_when_a_parent_folder_is_a_file(
    tmp_path: Path,
) -> None:
    """A file where a folder should be: `path_not_writable`, `not_a_directory`."""
    (tmp_path / "plain").write_text("a file")
    path = tmp_path / "plain" / "inner" / "report.pdf"

    error = await failure(write_file_whole(path, b"x", call="save_as"))

    assert isinstance(error, PathNotWritableError)
    assert (
        error.fix
        == "Give `save_as` a path whose parent folders are folders, not files."
    )
    assert error.details["reason"] == "not_a_directory"
    assert isinstance(error.__cause__, OSError)


async def test_leaves_the_path_as_it_was_when_cancelled_mid_write(
    tmp_path: Path,
) -> None:
    """A deadline mid-write removes the temporary file."""
    path = tmp_path / "data.csv"
    path.write_text("what it held")

    async def endless() -> AsyncIterator[bytes]:
        yield b"first part\n"
        await anyio.sleep_forever()
        yield b"never"

    with anyio.move_on_after(0.2):
        await write_file_whole(path, endless(), call="save_as")

    assert path.read_text() == "what it held"
    assert names_in(tmp_path) == ["data.csv"]


async def test_abort_removes_the_temporary_file(tmp_path: Path) -> None:
    """An aborted write leaves the path as it was."""
    path = tmp_path / "data.csv"
    path.write_text("what it held")

    saving = await FileSaving.create(path, call="save_as")
    await saving.write(b"half a file")
    assert len([name for name in names_in(tmp_path) if name.endswith(".partial")]) == 1
    await saving.abort()

    assert path.read_text() == "what it held"
    assert names_in(tmp_path) == ["data.csv"]


@contextlib.contextmanager
def small_filesystem(folder: Path, size: int) -> Generator[Path]:
    """A tmpfs of `size` bytes over `folder`, as this process reaches it."""
    folder.mkdir()
    unshare = TEST_UNSHARE if Path(TEST_UNSHARE).exists() else shutil.which("unshare")
    assert unshare is not None, "no `unshare`: run `ci/os-services.sh setup`"
    sleeper = subprocess.Popen(
        [
            unshare,
            "--user",
            "--map-root-user",
            "--mount",
            "sh",
            "-c",
            'mount -t tmpfs -o size="$2" tmpfs "$1" && touch "$1/.mounted" '
            "&& exec sleep 600",
            "sh",
            str(folder),
            str(size),
        ],
        stdin=subprocess.DEVNULL,
    )
    try:
        inside = Path("/proc", str(sleeper.pid), "root", *folder.parts[1:])
        marker = inside / ".mounted"
        for _ in range(500):
            if marker.exists():
                marker.unlink()
                break
            subprocess.run(["sleep", "0.01"], check=True)
        else:
            pytest.fail(
                f"`{unshare}` mounted no tmpfs over {folder}: "
                "run `ci/os-services.sh setup`"
            )
        yield inside
    finally:
        sleeper.kill()
        sleeper.wait()


@pytest.mark.skipif(
    sys.platform != "linux", reason="a tmpfs in a user namespace is Linux's"
)
async def test_raises_file_write_failed_on_a_full_filesystem_keeping_the_file(
    tmp_path: Path,
) -> None:
    """A full filesystem: `file_write_failed`, side `program`, nothing replaced."""
    with small_filesystem(tmp_path / "small", 64 * 1024) as inside:
        path = inside / "data.bin"
        path.write_text("what it held")

        error = await failure(write_file_whole(path, bytes(256 * 1024), call="save_as"))

        assert isinstance(error, FileWriteFailedError)
        assert (error.code, error.retryable) == ("file_write_failed", False)
        assert error.details == {
            "reason": "storage_full",
            "os_code": "ENOSPC",
            "path": str(path),
            "side": "program",
        }
        assert isinstance(error.__cause__, OSError)
        assert path.read_text() == "what it held"
        assert names_in(inside) == ["data.bin"]


class PlayedEngine(ByteStream):
    """The engine's side of an in-memory transport the test plays."""

    def __init__(self) -> None:
        """Opens both directions."""
        self._to_client_send, self._to_client = anyio.create_memory_object_stream[
            bytes
        ](max_buffer_size=1024)
        self._from_client_send, self._from_client = anyio.create_memory_object_stream[
            bytes
        ](max_buffer_size=1024)
        self._pending = b""

    # The Client's side.
    async def receive(self, max_bytes: int = 65536) -> bytes:
        """The engine's next bytes."""
        del max_bytes
        return await self._to_client.receive()

    async def send(self, item: bytes) -> None:
        """Takes the Client's bytes."""
        await self._from_client_send.send(item)

    async def send_eof(self) -> None:
        """Closes the Client's end; the engine's ends with it."""
        await self._from_client_send.aclose()
        await self._to_client_send.aclose()

    async def aclose(self) -> None:
        """Closes both ends."""
        await self.send_eof()

    # The engine's side.
    async def next(self) -> dict[str, object]:
        """The next message the Client wrote."""
        while b"\n" not in self._pending:
            self._pending += await self._from_client.receive()
        line, self._pending = self._pending.split(b"\n", 1)
        message: dict[str, object] = json.loads(line)
        return message

    async def play(self, message: dict[str, object]) -> None:
        """Sends the Client one message."""
        await self._to_client_send.send(
            json.dumps({"jsonrpc": "2.0", **message}).encode() + b"\n"
        )


async def connected(engine: PlayedEngine) -> ClientConnection:
    """A Connection to `engine`, past its hello."""
    connection: ClientConnection | None = None

    async def open_it() -> None:
        nonlocal connection
        connection = await ClientConnection.open(
            engine, needs=Product.collie, error_classes=ERROR_CLASSES
        )

    async with anyio.create_task_group() as group:
        group.start_soon(open_it)
        hello = await engine.next()
        await engine.play(
            {
                "id": hello["id"],
                "result": {
                    "protocol_version": "0.1",
                    "product": "collie",
                    "product_version": "0.1.0",
                    "engine_instance": "0123456789abcdef",
                },
            }
        )
    assert connection is not None
    return connection


def chunk(call_id: object, offset: int, data: bytes) -> dict[str, object]:
    """A `$/chunk` notification of the call `call_id`."""
    return {
        "method": "$/chunk",
        "params": {
            "id": call_id,
            "offset": offset,
            "data": base64.b64encode(data).decode(),
        },
    }


async def test_gives_a_local_engine_the_absolute_path_and_writes_nothing_when_it_wrote(
    tmp_path: Path,
) -> None:
    """A local engine gets the absolute path; its write is the file."""
    engine = PlayedEngine()
    connection = await connected(engine)
    path = tmp_path / "report.pdf"
    result: object = None
    call: dict[str, object] = {}
    credit: dict[str, object] = {}

    async def save() -> None:
        nonlocal result
        result = await save_file(
            connection, "download.save_as", {"download_id": "d1"}, path, call="save_as"
        )

    async with anyio.create_task_group() as group:
        group.start_soon(save)
        call = await engine.next()
        credit = await engine.next()
        await engine.play({"id": call["id"], "result": {"written": True}})

    assert result == {"written": True}
    assert call["method"] == "download.save_as"
    assert call["params"] == {"download_id": "d1", "path": str(path)}
    assert credit["params"] == {"id": call["id"], "credits": 16}
    assert names_in(tmp_path) == []
    await connection.close()


async def test_writes_the_chunks_the_engine_streams_when_it_did_not_write(
    tmp_path: Path,
) -> None:
    """The chunks the engine streams under credits are written whole."""
    engine = PlayedEngine()
    connection = await connected(engine)
    path = tmp_path / "out" / "data.csv"

    async def save() -> None:
        await save_file(
            connection, "download.save_as", {"download_id": "d1"}, path, call="save_as"
        )

    async with anyio.create_task_group() as group:
        group.start_soon(save)
        call = await engine.next()
        await engine.next()
        await engine.play(chunk(call["id"], 0, b"a,b\n"))
        await engine.play(chunk(call["id"], 4, b"1,2\n"))
        # Each chunk written grants one more credit.
        assert (await engine.next())["params"] == {"id": call["id"], "credits": 1}
        assert (await engine.next())["params"] == {"id": call["id"], "credits": 1}
        await engine.play({"id": call["id"], "result": {"written": False}})

    assert path.read_bytes() == b"a,b\n1,2\n"
    assert names_in(path.parent) == ["data.csv"]
    await connection.close()


async def test_leaves_the_path_as_it_was_when_the_engine_fails_the_call_mid_stream(
    tmp_path: Path,
) -> None:
    """The engine's failure mid-stream leaves the path as it was."""
    engine = PlayedEngine()
    connection = await connected(engine)
    path = tmp_path / "data.csv"
    path.write_text("what it held")
    error: BaseException | None = None

    async def save() -> None:
        nonlocal error
        error = await failure(
            save_file(
                connection,
                "download.save_as",
                {"download_id": "d1"},
                path,
                call="save_as",
            )
        )

    async with anyio.create_task_group() as group:
        group.start_soon(save)
        call = await engine.next()
        await engine.next()
        await engine.play(chunk(call["id"], 0, b"half"))
        await engine.next()
        await engine.play(
            {
                "id": call["id"],
                "error": {
                    "code": -32000,
                    "message": "the page crashed",
                    "data": {"code": "page_crashed", "retryable": True},
                },
            }
        )

    assert getattr(error, "code", None) == "page_crashed"
    assert path.read_text() == "what it held"
    assert names_in(tmp_path) == ["data.csv"]
    await connection.close()
