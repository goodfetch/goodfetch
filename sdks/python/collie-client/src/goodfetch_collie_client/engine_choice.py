"""Which engine `connect()` reaches (ADRs 0024, 0078, 0083 and 0098).

And the bytes to it when it is a Daemon. `engine` is `auto` unless code
sets it: then the remote Daemon `GOODFETCH_URL` names with
`GOODFETCH_TOKEN`'s token, else the Daemon of the SDK's line whose runtime
file is in the Goodfetch home and answers, else an Attached engine.
`attached` and `daemon` choose one of the two, and `RemoteDaemon(url,
token)` a remote Daemon, whatever the environment says; there is no
`GOODFETCH_ENGINE`. A remote Daemon's address is `ws://` on loopback,
reached over WebSocket with the subprotocol `goodfetch.v<major>` (ADR 0049;
spec #606's Transports): `wss://` and addresses beyond loopback wait for the
Daemon's TLS, so a token never crosses the network in the clear. A token
goes into `hello` alone, never into a log, an error or `EngineInfo` (ADR
0067). TypeScript's `engine-choice.ts` is its twin.
"""

from __future__ import annotations

import ipaddress
import json
import os
import subprocess
import sys
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Literal, TypeAlias, cast, overload
from urllib.parse import urlsplit

import anyio

from goodfetch_collie_client.attached_engine import is_executable_file
from goodfetch_collie_client.generated.connection import (
    PRODUCT_VERSION,
    WEBSOCKET_SUBPROTOCOL,
)
from goodfetch_collie_client.generated.errors import (
    DaemonUnreachableError,
    EngineNotFoundError,
    SettingInvalidError,
)
from goodfetch_collie_client.websocket_transport import open_websocket

if TYPE_CHECKING:
    from collections.abc import Callable, Mapping

    from anyio.abc import ByteStream, SocketStream

    from goodfetch_collie_client.product_connection import EngineLocation

if sys.platform != "win32":
    # The OS's user database, where `dirs` finds a home when `HOME` is unset.
    import pwd

URL_VARIABLE = "GOODFETCH_URL"
"""The variable naming a remote Daemon, read at `auto` alone (ADR 0083)."""

TOKEN_VARIABLE = "GOODFETCH_TOKEN"
"""The variable holding the remote Daemon's token."""

HOME_VARIABLE = "GOODFETCH_HOME"
"""The variable naming the Goodfetch home."""


@dataclass(frozen=True)
class RemoteDaemon:
    """A remote Daemon, Exposed or Goodfetch Cloud, and the token its `hello` takes."""

    url: str
    """Its address: `ws://`, a loopback IP address or `localhost`, and a port.

    Such as `ws://127.0.0.1:7878`.
    """
    token: str = field(repr=False)
    """The token its `hello` takes, which nothing the SDK shows holds."""


EngineChoice: TypeAlias = Literal["auto", "attached", "daemon"] | RemoteDaemon
"""The engine `connect()` reaches.

`auto`, the default, takes the remote Daemon `GOODFETCH_URL` names, else a
running Daemon, else an Attached engine; `attached` always starts an
Attached engine; `daemon` reaches the Daemon of the SDK's line, starting it
when none runs; `RemoteDaemon(url, token)` reaches that remote Daemon.
"""


@dataclass(frozen=True)
class AttachedReach:
    """An Attached engine is to be started."""

    home: str | None
    """The Goodfetch home it runs in, or `None` when the OS names none."""


@dataclass(frozen=True)
class RuntimeFile:
    """The runtime file a Daemon of this machine was reached by."""

    home: str
    """The Goodfetch home it is in."""
    text: str = field(repr=False)
    """What it held, its token included."""


@dataclass(frozen=True)
class DaemonReach:
    """A Daemon reached, before `hello`."""

    url: str
    """Where it listens: a remote Daemon's URL, a socket path or a loopback address."""
    stream: ByteStream
    """The bytes to it, open: its socket's, or a remote Daemon's WebSocket's."""
    token: str = field(repr=False)
    """The token its `hello` carries."""
    closed_at_hello: Callable[[], DaemonUnreachableError]
    """What fails `connect()` when the Daemon closes before it answers."""
    runtime_file: RuntimeFile | None
    """The runtime file this machine's Daemon was reached by.

    By it the Daemon it hands over to is told from it (ADR 0098); `None`
    for a remote Daemon, which never hands over.
    """

    @property
    def kind(self) -> Literal["daemon", "remote"]:
        """This machine's Daemon, or a remote one."""
        return "remote" if self.runtime_file is None else "daemon"

    @property
    def home(self) -> str | None:
        """The Daemon's home: `None` for a remote one, whose home is its machine's."""
        return None if self.runtime_file is None else self.runtime_file.home


Reach: TypeAlias = AttachedReach | DaemonReach
"""How the chosen engine is reached, before `hello`."""


async def reach_engine(
    choice: EngineChoice,
    env: Mapping[str, str],
    executable: Callable[[], EngineLocation],
) -> Reach:
    """Chooses the engine `choice` names, and opens the bytes to a Daemon.

    `GOODFETCH_URL` and `GOODFETCH_TOKEN` are read from `env` at `auto`
    alone. `executable` gives the executable a Daemon is started with, when
    `choice` is `daemon` and none runs, and the fix `engine_not_found` gives
    when it is missing.

    Raises:
        SettingInvalidError: `choice` is no engine choice, a remote Daemon's
            URL names no loopback `ws://` address, or `GOODFETCH_URL` comes
            without `GOODFETCH_TOKEN`.
        EngineNotFoundError: `choice` is `daemon`, none runs, and its
            executable is missing.
        DaemonUnreachableError: Nothing answers where a remote Daemon's URL
            says, or a Daemon cannot be started or its runtime file read.
    """
    if isinstance(choice, RemoteDaemon):
        return await _reach_remote(choice, "engine")
    if choice == "attached":
        return AttachedReach(_home_of(env))
    if choice == "daemon":
        return await _reach_daemon(env, executable)
    if choice == "auto":
        remote = _remote_of_environment(env)
        if remote is not None:
            return await _reach_remote(remote, URL_VARIABLE)
        home = _home_of(env)
        daemon = None if home is None else await _running_daemon(home)
        return AttachedReach(home) if daemon is None else daemon
    # A caller without a type checker may pass anything.
    unknown: object = choice
    raise SettingInvalidError(
        "engine names no engine choice",
        fix=("Give engine `auto`, `attached`, `daemon` or a RemoteDaemon(url, token)."),
        details={
            "key": "engine",
            "reason": f"{type(unknown).__name__} is no engine choice",
        },
    )


def _remote_of_environment(env: Mapping[str, str]) -> RemoteDaemon | None:
    """The remote Daemon `GOODFETCH_URL` names with `GOODFETCH_TOKEN`'s token.

    `None` when it is unset. Its URL is judged first, as the CLI judges it.
    """
    url = _variable(env, URL_VARIABLE)
    if url is None:
        return None
    _remote_address(url, URL_VARIABLE)
    token = _variable(env, TOKEN_VARIABLE)
    if token is None:
        raise SettingInvalidError(
            f"{URL_VARIABLE} names a Daemon, "
            f"and {TOKEN_VARIABLE} holds no token for it",
            fix=(
                f"Set {TOKEN_VARIABLE} to the token the Daemon {URL_VARIABLE} names "
                f"takes, or unset {URL_VARIABLE} to use this machine's engine."
            ),
            details={
                "key": TOKEN_VARIABLE,
                "reason": f"{URL_VARIABLE} names a Daemon, and it holds no token",
            },
        )
    return RemoteDaemon(url, token)


async def _reach_remote(remote: RemoteDaemon, source: str) -> DaemonReach:
    """Opens the bytes to the remote Daemon `remote` names, which `source` gave.

    `source` is `engine` or `GOODFETCH_URL`.
    """
    url = remote.url
    host, port, origin = _remote_address(url, source)
    socket = await _open_tcp(host, port)
    stream = (
        None if socket is None else await open_websocket(socket, url, origin=origin)
    )
    if stream is None:
        # A WebSocket tells none of these apart: a Daemon of another protocol
        # major answers no subprotocol this Client offered, so its handshake
        # fails before `hello` could name the versions.
        raise _unreachable(
            f"No Daemon at {url}, which {source} names, opened a WebSocket of "
            f"subprotocol {WEBSOCKET_SUBPROTOCOL}: nothing listens there, it "
            "refused the handshake, or it speaks another protocol major",
            f"Start the Daemon {url} names with an engine of this SDK's Product "
            f"version {PRODUCT_VERSION}, reach it through a forwarder that "
            "listens on loopback, such as an SSH tunnel, or leave engine at auto "
            f"with {URL_VARIABLE} unset to use this machine's engine.",
        )
    fix = (
        f"Set {TOKEN_VARIABLE} to the token the Daemon at {url} takes."
        if source == URL_VARIABLE
        else f"Give engine the token the Daemon at {url} takes."
    )
    return DaemonReach(
        url=url,
        stream=stream,
        token=remote.token,
        closed_at_hello=lambda: _unreachable(_closed_at_hello(url), fix),
        runtime_file=None,
    )


def _remote_address(url: str, source: str) -> tuple[str, int, str]:
    """The address a remote Daemon's URL names, and its origin.

    `ws://` with a loopback IP address or `localhost` and a port, and
    nothing else, so no credential in it reaches a log, an error or
    `EngineInfo`. The origin is what its WebSocket handshake sends, as a
    Client outside a browser does (ADR 0049).

    Raises:
        SettingInvalidError: Naming `source` otherwise, without the URL when
            it is no URL or holds more than an address, which may be a secret.
    """

    def refused(why: str, *, shown: bool = True) -> SettingInvalidError:
        what = f"{url} {why}" if shown else why
        return SettingInvalidError(
            f"{source} {what}",
            fix=(
                f"Give {source} a Daemon's address on loopback, "
                "ws://<address>:<port>, such as ws://127.0.0.1:7878: a Daemon "
                "beyond loopback is reached through a forwarder that listens on "
                "it, such as an SSH tunnel, until the Daemon terminates TLS."
            ),
            details={"key": source, "reason": what},
        )

    try:
        parsed = urlsplit(url)
        port = parsed.port
    except ValueError:
        raise refused("is no URL", shown=False) from None
    if not parsed.scheme or not parsed.netloc:
        raise refused("is no URL", shown=False)
    if (
        parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
    ):
        raise refused(
            "holds more than an address: a Daemon's address has no user, "
            "password, query or fragment, and its token goes in its own setting",
            shown=False,
        )
    if parsed.scheme != "ws":
        raise refused(
            "names no Daemon: give ws://<address>:<port>; wss:// waits for the "
            "Daemon's TLS"
        )
    if port is None or parsed.path not in ("", "/"):
        raise refused(
            "has no port, or has a path, where a Daemon's address has a port alone"
        )
    hostname = parsed.hostname or ""
    host = "127.0.0.1" if hostname == "localhost" else hostname
    if not _is_loopback(host):
        raise refused(
            "names an address beyond loopback, where its token would cross the "
            "network in the clear until the Daemon terminates TLS"
        )
    return host, port, f"http://{parsed.netloc}"


def _is_loopback(host: str) -> bool:
    """Whether `host`, an IP address, is a loopback one."""
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False


async def _reach_daemon(
    env: Mapping[str, str], executable: Callable[[], EngineLocation]
) -> DaemonReach:
    """The Daemon of the SDK's line, started by `daemon start` if none runs (ADR 0024).

    Raises:
        DaemonUnreachableError: It cannot be started or reached.
    """
    home = _home_of(env)
    if home is None:
        raise _unreachable(
            "No Goodfetch home names where the Daemon is found",
            f"Set {HOME_VARIABLE} to the Goodfetch home, "
            "or a home folder the OS knows.",
        )
    running = await _running_daemon(home)
    if running is not None:
        return running
    location = executable()
    await _start_daemon(location, env)
    started = await _running_daemon(home)
    if started is None:
        raise _unreachable(
            f"The Daemon {location.path} started is not answering",
            f"Start it with `{location.path} daemon start`, and find why it "
            "stopped in its log.",
        )
    return started


async def _start_daemon(location: EngineLocation, env: Mapping[str, str]) -> None:
    """Runs `daemon start --json` of `location` with `env`.

    It returns once the Daemon serves; a refused start fails with the
    message and fix it printed.
    """
    path = location.path

    def not_found() -> EngineNotFoundError:
        return EngineNotFoundError(
            f"no engine executable at {path}",
            fix=location.not_found_fix,
            details={"path": path},
        )

    if not await is_executable_file(path):
        raise not_found()
    try:
        started = await anyio.run_process(
            [path, "daemon", "start", "--json"],
            check=False,
            stdin=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            env=dict(env),
            start_new_session=True,
        )
    except (FileNotFoundError, PermissionError) as cause:
        raise not_found() from cause
    except OSError as cause:
        raise _unreachable(
            f"The Daemon could not be started with {path}",
            f"Start it with `{path} daemon start`, or leave engine at auto to use "
            "an Attached engine.",
        ) from cause
    if started.returncode != 0:
        refusal = _refusal_of(started.stdout)
        message, fix = (None, None) if refusal is None else refusal
        raise _unreachable(
            f"The Daemon could not be started: `{path} daemon start` exited with "
            f"{started.returncode}" + ("" if message is None else f": {message}"),
            fix
            or f"Start it with `{path} daemon start`, whose error says why it "
            "cannot, or leave engine at auto to use an Attached engine.",
        )


def _refusal_of(printed: bytes) -> tuple[str, str | None] | None:
    """The message and fix of the `CommandFailed` a command printed under `--json`.

    `None` when it printed none.
    """
    try:
        parsed: object = json.loads(printed)
    except ValueError:
        return None
    error = _member(parsed, "error")
    message = _member(error, "message")
    if not isinstance(message, str):
        return None
    fix = _member(error, "fix")
    return message, fix if isinstance(fix, str) else None


HANDOVER_POLL = 0.05
"""How often, in seconds, the wait for the Daemon an idle one hands over to
reads the runtime file."""


@overload
async def handed_over_to(handed_over: RuntimeFile, until: None) -> DaemonReach: ...
@overload
async def handed_over_to(
    handed_over: RuntimeFile, until: float
) -> DaemonReach | None: ...
async def handed_over_to(
    handed_over: RuntimeFile, until: float | None
) -> DaemonReach | None:
    """The Daemon serving in `handed_over`'s home once another handed over (ADR 0098).

    The runtime file is read again every 50 ms, the one whose text is
    `handed_over`'s skipped, until a Daemon answers; `None` once `until`, on
    anyio's clock, passed. With no `until`, it waits until cancelled.

    Raises:
        DaemonUnreachableError: The runtime file cannot be read.
    """
    while True:
        pick = await _running_daemon(handed_over.home, skipped=handed_over.text)
        if pick is not None:
            return pick
        if until is not None and anyio.current_time() >= until:
            return None
        await anyio.sleep(HANDOVER_POLL)


async def _running_daemon(
    home: str, *, skipped: str | None = None
) -> DaemonReach | None:
    """The Daemon whose runtime file is in `home`'s folder of the SDK's line.

    When its endpoint answers; `None` when there is no runtime file, its
    text is `skipped`, it holds no endpoint and token, as a dead Daemon's
    half-written or older one, or nothing listens where it says, as for a
    Daemon that died. Whether the endpoint answers says whether it runs, as
    the TypeScript SDK, which cannot ask the Daemon's lock, finds it.

    Raises:
        DaemonUnreachableError: The runtime file cannot be read.
    """
    file = os.path.join(home, _line_of(PRODUCT_VERSION), "daemon.json")
    try:
        text = await anyio.Path(file).read_text("utf-8")
    except FileNotFoundError:
        return None
    except (OSError, UnicodeDecodeError) as cause:
        raise _unreadable(file) from cause
    if text == skipped:
        return None
    runtime = _runtime_file_of(text)
    if runtime is None:
        return None
    endpoint, token = runtime
    kind, address = endpoint
    if kind == "unix":
        stream = await _open_unix(address)
    else:
        host, _, port = address.rpartition(":")
        stream = (
            await _open_tcp(host.removeprefix("[").removesuffix("]"), int(port))
            if port.isdigit()
            else None
        )
    if stream is None:
        return None
    fix = (
        f"Restart it with `daemon restart`, which writes a new runtime file in {home}."
    )
    return DaemonReach(
        url=address,
        stream=stream,
        token=token,
        closed_at_hello=lambda: _unreachable(_closed_at_hello(address), fix),
        runtime_file=RuntimeFile(home, text),
    )


def _closed_at_hello(url: str) -> str:
    return (
        f"The Daemon at {url} closed the Connection before it answered hello, "
        "as one does for a token it does not take"
    )


def _unreachable(message: str, fix: str) -> DaemonUnreachableError:
    """A Daemon nothing reaches: `daemon_unreachable`, its fix in its details too."""
    return DaemonUnreachableError(message, fix=fix, details={"fix": fix})


def _unreadable(file: str) -> DaemonUnreachableError:
    """A runtime file that cannot be read fails `connect()`, as in the CLI."""
    return _unreachable(
        f"The Daemon's runtime file {file} cannot be read",
        f"Stop the Daemon with `daemon stop`, or remove {file} when none runs, "
        "then connect again.",
    )


def _runtime_file_of(
    text: str,
) -> tuple[tuple[Literal["unix", "tcp"], str], str] | None:
    """The endpoint and token of a runtime file's text; `None` when it holds none."""
    try:
        parsed: object = json.loads(text)
    except ValueError:
        return None
    token = _member(parsed, "token")
    endpoint = _member(parsed, "endpoint")
    if not isinstance(token, str):
        return None
    unix = _member(endpoint, "unix")
    if isinstance(unix, str):
        return ("unix", unix), token
    tcp = _member(endpoint, "tcp")
    if isinstance(tcp, str):
        return ("tcp", tcp), token
    return None


def _member(value: object, name: str) -> object:
    """`value`'s member `name` when it is a JSON object; `None` otherwise."""
    if not isinstance(value, dict):
        return None
    return cast("dict[str, object]", value).get(name)


async def _open_unix(path: str) -> SocketStream | None:
    """A connected stream to the Unix socket `path`; `None` when nothing listens there.

    Raises:
        DaemonUnreachableError: For any other error than nothing listening.
    """
    try:
        return await anyio.connect_unix(path)
    except (ConnectionRefusedError, FileNotFoundError):
        return None
    except OSError as cause:
        raise _unopened() from cause


async def _open_tcp(host: str, port: int) -> SocketStream | None:
    """A connected stream to `host:port`; `None` when nothing listens there.

    Raises:
        DaemonUnreachableError: For any other error than nothing listening.
    """
    try:
        return await anyio.connect_tcp(host, port)
    except OSError as error:
        # anyio raises its own error, caused by each attempt's.
        if isinstance(error, ConnectionRefusedError) or isinstance(
            error.__cause__, ConnectionRefusedError
        ):
            return None
        raise _unopened() from error


def _unopened() -> DaemonUnreachableError:
    return _unreachable(
        "The Daemon's endpoint could not be opened",
        "Check that the Daemon's socket is yours and reachable, or connect with "
        "engine attached.",
    )


def _home_of(env: Mapping[str, str]) -> str | None:
    """The Goodfetch home.

    `GOODFETCH_HOME` when it is set, else the `goodfetch` folder of the OS's
    usual data folder, as the engine finds it (`dirs`'s `data_dir`); `None`
    when the environment names no home folder.
    """
    named = _variable(env, HOME_VARIABLE)
    if named is not None:
        return named
    data = _data_folder(env)
    return None if data is None else os.path.join(data, "goodfetch")


def _data_folder(env: Mapping[str, str]) -> str | None:
    """The OS's usual data folder for the user `env` is for.

    From its `HOME`, else the user's home in the OS's user database, as
    `dirs` finds it, never the process's own environment.
    """
    if sys.platform == "win32":
        return _variable(env, "APPDATA")
    home = _variable(env, "HOME") or _user_home()
    if home is None:
        return None
    if sys.platform == "darwin":
        return os.path.join(home, "Library", "Application Support")
    xdg = _variable(env, "XDG_DATA_HOME")
    return (
        xdg
        if xdg is not None and os.path.isabs(xdg)
        else os.path.join(home, ".local", "share")
    )


def _user_home() -> str | None:
    """The user's home in the OS's user database; `None` when it has none."""
    if sys.platform == "win32":
        return None
    try:
        home = pwd.getpwuid(os.getuid()).pw_dir
    except KeyError:
        return None
    return home or None


def _line_of(version: str) -> str:
    """The per-major folder's name of `version` (ADR 0030).

    Its major, or `0.y` during previews.
    """
    parts = version.replace("+", ".").replace("-", ".").split(".")
    major = parts[0] or "0"
    minor = parts[1] if len(parts) > 1 and parts[1] else "0"
    return f"0.{minor}" if major == "0" else major


def _variable(env: Mapping[str, str], name: str) -> str | None:
    """The value of `name` in `env`.

    `None` when it is unset or empty, as the engine reads a deployment variable.
    """
    value = env.get(name)
    return value or None
