"""What a Product's package builds its Connection type on (ADRs 0024, 0090 and 0095).

It reaches the engine `engine` chooses: a Daemon, or the Product's
executable, the one its engine-path variable names or else the one its
platform wheel holds, started as an Attached engine with exactly the
program's environment map. Then it finishes `hello`, naming the Product,
and logs the engine reached. The other Product's variable is never read
here. TypeScript's `product-connection.ts` is its twin.
"""

from __future__ import annotations

import logging
import os
import sys
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import TYPE_CHECKING

import anyio
import anyio.to_thread

from goodfetch_collie_client._deployment_variables import check_deployment_variables
from goodfetch_collie_client._installed_executables import (
    PickedClient,
    would_be_the_pick,
)
from goodfetch_collie_client._project_file import read_project
from goodfetch_collie_client._relink import Reconnect
from goodfetch_collie_client._rpc import ChannelBounds, bounds_of, phase_exceeded
from goodfetch_collie_client.attached_engine import AttachedEngine, is_executable_file
from goodfetch_collie_client.connection import (
    CLOSE_TIMEOUT,
    ClientConnection,
    DaemonHello,
    EngineFallback,
    EngineInfo,
    Reached,
    connect_limit,
)
from goodfetch_collie_client.engine_choice import (
    AttachedReach,
    handed_over_to,
    reach_engine,
)
from goodfetch_collie_client.generated.connection import PRODUCT_VERSION
from goodfetch_collie_client.goodfetch_error import GoodfetchError
from goodfetch_collie_client.handle import NotGiven, not_given, timeout_of
from goodfetch_collie_client.host_logging import CLIENT_LOGGER
from goodfetch_collie_client.limits import limit_list
from goodfetch_collie_client.listeners import MAX_LISTENER_QUEUE, listener_queue_of

if TYPE_CHECKING:
    from collections.abc import Mapping, Sequence
    from datetime import timedelta
    from pathlib import Path

    from anyio.abc import ByteStream

    from goodfetch_collie_client._project_file import ConnectTimeout, ProjectFile
    from goodfetch_collie_client._relink import OperationClass
    from goodfetch_collie_client.engine_choice import (
        DaemonReach,
        EngineChoice,
        RuntimeFile,
    )
    from goodfetch_collie_client.generated.models import Product, RunLimit
    from goodfetch_collie_client.goodfetch_error import ErrorClass
    from goodfetch_collie_client.limits import Limits

__all__ = ["ProductPackage", "open_engine", "open_over"]


@dataclass(frozen=True)
class ProductPackage:
    """What tells one Product's package from the other's when it connects."""

    needs: Product
    """The Product its Client needs, which `hello` names."""
    name: str
    """The Product's name in an error's message and `fix`, such as `Collie`."""
    package: str
    """Its platform wheel's distribution, which holds its engine."""
    executable: str
    """Its executable's name, without Windows's `.exe`."""
    engine_path_variable: str
    """Its engine-path variable, the only one read."""
    error_classes: Mapping[str, ErrorClass]
    """The error class of each code the Product knows."""
    bin_folder: Path
    """The folder of its platform wheel that holds its executable."""
    operation_classes: Mapping[str, OperationClass]
    """The class of each operation the Product serves (ADR 0011).

    A reconnect sends a call in flight again only when it is a read or an
    idempotent write (ADR 0079).
    """


@dataclass(frozen=True)
class EngineLocation:
    """Where to start a Product's engine, and what to do when it is not there."""

    path: str
    """The executable."""
    not_found_fix: str
    """The `fix` of `engine_not_found` when `path` is no executable."""


def locate_engine(env: Mapping[str, str], product: ProductPackage) -> EngineLocation:
    """The executable `env` names for `product`, or its platform wheel's.

    `env` has passed `check_deployment_variables`, so a variable it holds is
    never empty.
    """
    variable = product.engine_path_variable
    name, executable, package = product.name, product.executable, product.package
    named = env.get(variable)
    if named is not None:
        return EngineLocation(
            named,
            f"{variable} names {named}, which is no executable file: point it at "
            f"{name}'s executable, {executable}, or leave it unset to use the one "
            f"{package} installed.",
        )
    file = f"{executable}.exe" if sys.platform == "win32" else executable
    return EngineLocation(
        str(product.bin_folder / file),
        f"Install {package}'s platform wheel for this system, which holds its "
        f"engine, or set {variable} to {name}'s executable.",
    )


async def open_engine(
    product: ProductPackage,
    *,
    env: Mapping[str, str] | None = None,
    engine: EngineChoice = "auto",
    close_timeout: float = CLOSE_TIMEOUT,
    limits: Limits | None = None,
    connect_timeout: timedelta | NotGiven | None = not_given,
    max_listener_queue: int = MAX_LISTENER_QUEUE,
    settings: Mapping[str, object] | None = None,
    project: str | os.PathLike[str] | NotGiven | None = not_given,
) -> ClientConnection:
    """Reaches the engine `engine` chooses for `product`, then `hello` and `limits`.

    `engine` is `auto` unless given: the remote Daemon `GOODFETCH_URL` names,
    else the running Daemon of the SDK's line, else an Attached engine (ADR
    0024). `env` is the environment map, read here and never at import: the
    process's own when left out (ADRs 0062 and 0083). An Attached engine,
    and a Daemon this call starts, gets exactly this map. A `GOODFETCH_`
    variable off the closed list of deployment variables
    (`setting_unknown`, naming the closest) or one set but empty
    (`setting_invalid`) raises before any engine is reached. `connect_timeout`
    is how long `hello` waits for its answer: `timeout.connect`'s 30 s when
    left out, none for `None`; past it, `DeadlineExceededError` naming
    `timeout.connect` raises. `max_listener_queue` bounds the queue of each
    listener of the Connection's own Subscription. A `limits` amount the SDK
    cannot turn exactly, a `connect_timeout` of zero or less or a
    `max_listener_queue` that is no whole number of 1 or more
    (`setting_invalid`) raises before any engine is reached. A refused engine
    choice (`setting_invalid`), a missing executable (`engine_not_found`), a
    Daemon nothing reaches or that refuses the token (`daemon_unreachable`),
    a refused Product (`not_in_product`) or version, or a refused Limit
    raises here, with a `fix`, leaving no Attached engine behind. This
    machine's Daemon refusing by Product or version is waited past when it
    hands over to its major's pick, within `timeout.connect`; otherwise,
    under `auto`, an Attached engine runs in its place, `EngineInfo.fallback`
    saying why, logged once at warning with the record's `fallback`, and
    under `daemon` the refusal raises, its `fix` naming this executable's
    `daemon restart` (ADR 0098). The engine
    reached is logged at info on the `goodfetch.client` logger, its
    `EngineInfo` as the record's `engine`. `settings`, the Connection's own
    (ADR 0083), typed by `product`'s package, go in `hello`, and a key the
    engine refuses fails the opening
    with its error, such as `setting_not_applicable`. Their
    `connection.max_pending_calls` and `connection.max_queued_write_bytes`
    bound the Connection's own calls from `hello` on; one that is no whole
    number of 1 or more (`setting_invalid`) raises before any engine is
    reached. `project` is the project file to read (ADR 0083), sent as the
    project configuration of each Run the Connection starts and of its
    `explain_settings()`: the `goodfetch.toml` in the working directory or
    the nearest folder above it when not given, the file it names, a
    relative one from the working directory, or none for `None`. Its
    `timeout.connect` bounds `hello` when `connect_timeout` is not given,
    and `EngineInfo.project_file` names it. A named file that is not there,
    or one that does not read as a project file (`setting_invalid`), or
    one of a format version this Goodfetch cannot read
    (`format_version_unsupported`) raises before any engine is reached.
    """
    connection_limits = limit_list(limits)
    listener_queue = listener_queue_of(max_listener_queue)
    bounds = bounds_of(settings)
    checked = (
        connect_timeout
        if isinstance(connect_timeout, NotGiven)
        else timeout_of(connect_timeout, "timeout.connect")
    )
    environment = dict(os.environ if env is None else env)
    check_deployment_variables(environment)
    project_file = await anyio.to_thread.run_sync(read_project, project, os.getcwd)
    # The argument's `timeout.connect`, else the project file's, else the default.
    connect_bound: ConnectTimeout = (
        project_file.connect_timeout
        if isinstance(checked, NotGiven)
        and project_file is not None
        and project_file.connect_timeout is not None
        else checked
    )
    # A remote Daemon's handshake and `hello` share one `timeout.connect`.
    started_at = anyio.current_time()
    reach = await reach_engine(
        engine,
        environment,
        lambda: locate_engine(environment, product),
        connect_timeout=connect_bound,
        started_at=started_at,
    )
    opening = _Opening(
        close_timeout,
        connection_limits,
        connect_bound,
        listener_queue,
        settings,
        bounds,
        project_file,
    )
    if isinstance(reach, AttachedReach):
        connection = await _open_attached(
            product, environment, Reached("attached", reach.home, None), opening
        )
    else:
        connection = await _open_refusable(
            product,
            environment,
            reach,
            opening,
            # This machine's Daemon's `hello` counts from its reach, which may
            # have started it.
            started_at if reach.runtime_file is None else anyio.current_time(),
            auto=engine == "auto",
        )
    info = connection.engine
    logging.getLogger(CLIENT_LOGGER).info(
        _reached_text(product, info), extra={"engine": info}
    )
    return connection


@dataclass(frozen=True)
class _Opening:
    """What each `hello` of one `connect()` takes, whichever engine answers it."""

    close_timeout: float
    limits: Sequence[RunLimit]
    connect_timeout: ConnectTimeout
    listener_queue: int
    settings: Mapping[str, object] | None
    bounds: ChannelBounds
    project: ProjectFile | None


_REFUSALS = frozenset({"not_in_product", "protocol_version_unsupported"})
"""The codes of a Daemon's refusal of `hello` by Product or protocol version."""


async def _open_refusable(
    product: ProductPackage,
    env: Mapping[str, str],
    reach: DaemonReach,
    opening: _Opening,
    started_at: float,
    *,
    auto: bool,
) -> ClientConnection:
    """Finishes `hello` with the Daemon `reach` opened, or the pick it hands over to.

    `hello` and any wait for a pick end within `timeout.connect` of
    `started_at`, on anyio's clock. This machine's Daemon refusing by
    Product or protocol version (ADR 0098): when its refusal holds
    `handover`, the pick is waited for; otherwise, under `auto`, an
    Attached engine runs in its place, `EngineInfo.fallback` and one
    warning on `goodfetch.client` saying why, and under `daemon` the
    refusal raises, its `fix` adding this executable's `daemon restart`. A
    remote Daemon's refusal raises unchanged.
    """
    daemon = reach
    while True:
        try:
            return await _open_daemon(product, daemon, opening, started_at)
        except GoodfetchError as error:
            runtime_file = daemon.runtime_file
            if runtime_file is None or error.code not in _REFUSALS:
                raise
            if error.details.get("handover") is True:
                daemon = await _pick_of(runtime_file, started_at, opening)
                continue
            added = await _refusal_fix(product, env, runtime_file.home)
            fix = f"{error.fix or ''} {added}".strip()
            if not auto:
                error.fix = fix
                raise
            fallback = EngineFallback(
                cause=(
                    f"The Daemon at {daemon.url} refused this Client with "
                    f"{error.code}: {error}"
                ),
                fix=fix,
            )
            logging.getLogger(CLIENT_LOGGER).warning(
                "%s runs on an Attached engine instead of the Daemon at %s, which "
                "refused it with %s: %s %s",
                product.name,
                daemon.url,
                error.code,
                error,
                fix,
                extra={"fallback": fallback},
            )
            return await _open_attached(
                product,
                env,
                Reached("attached", daemon.home, None, fallback),
                opening,
            )


async def _pick_of(
    handed_over: RuntimeFile, started_at: float, opening: _Opening
) -> DaemonReach:
    """The pick the idle Daemon of `handed_over` hands over to, once it serves.

    Within `timeout.connect` of `started_at`, on anyio's clock.

    Raises:
        DeadlineExceededError: No pick served in time, naming
            `timeout.connect` and `waited_for` `hello`.
    """
    connect = connect_limit(opening.connect_timeout)
    if connect is None:
        return await handed_over_to(handed_over, None)
    value, limit = connect
    pick = await handed_over_to(handed_over, started_at + value / 1000)
    if pick is not None:
        return pick
    raise phase_exceeded(
        f"timeout.connect ({value} ms) passed before the Daemon an idle one "
        "handed over to served",
        fix=(
            "Raise connect()'s timeout, or find why the Daemon did not start in "
            f"its log in {handed_over.home}."
        ),
        limit=limit,
        started_at=started_at,
        waited_for="hello",
    )


async def _refusal_fix(
    product: ProductPackage, env: Mapping[str, str], home: str
) -> str:
    """What a refusal's `fix` adds for `product`'s Client (ADR 0098).

    As the engine words it: its executable's `daemon restart`, given by its
    path, and, where that executable would still not be its major's pick by
    the executables installed in `home`, Foxhound at this version or newer.
    With no executable at its path, it is the fix that installs one.
    """
    location = locate_engine(env, product)
    path = location.path
    if not await is_executable_file(path):
        return location.not_found_fix
    shown = f'"{path}"' if any(character.isspace() for character in path) else path
    restart = f"Run `{shown} daemon restart` to restart the Daemon on its major's pick."
    client = PickedClient(product.needs, PRODUCT_VERSION)
    now = datetime.now(UTC)
    if await anyio.to_thread.run_sync(would_be_the_pick, client, home, env, now):
        return restart
    return (
        f"{restart} {product.name}'s executable would still not run the Daemon "
        "while a Foxhound of the major is installed: install Foxhound "
        f"{PRODUCT_VERSION} or newer (`pip install goodfetch-foxhound`) and run "
        "its `daemon restart`."
    )


async def _open_attached(
    product: ProductPackage,
    env: Mapping[str, str],
    reached: Reached,
    opening: _Opening,
) -> ClientConnection:
    """Starts `product`'s engine as an Attached engine with `env`, then `hello`."""
    location = locate_engine(env, product)
    engine = await AttachedEngine.start(location.path, env, location.not_found_fix)
    return await ClientConnection.open(
        engine.transport,
        needs=product.needs,
        error_classes=product.error_classes,
        engine=engine,
        reached=reached,
        close_timeout=opening.close_timeout,
        limits=opening.limits,
        connect_timeout=opening.connect_timeout,
        listener_queue=opening.listener_queue,
        settings=opening.settings,
        bounds=opening.bounds,
        project=opening.project,
    )


async def _open_daemon(
    product: ProductPackage,
    reach: DaemonReach,
    opening: _Opening,
    connect_started_at: float,
) -> ClientConnection:
    """Finishes `hello` with a Daemon, its token in it.

    Within `timeout.connect` of `connect_started_at`. One that closes the
    Connection before it answers fails with `daemon_unreachable`. A remote
    Daemon, which `reach` can open again, is reached again once its link
    drops (spec 606).
    """
    reconnect = (
        None
        if reach.reopen is None
        else Reconnect(product.operation_classes, reach.reopen)
    )
    return await ClientConnection.open(
        reach.stream,
        needs=product.needs,
        error_classes=product.error_classes,
        reached=Reached(reach.kind, reach.home, reach.url),
        daemon=DaemonHello(reach.token, reach.closed_at_hello),
        close_timeout=opening.close_timeout,
        limits=opening.limits,
        connect_timeout=opening.connect_timeout,
        connect_started_at=connect_started_at,
        listener_queue=opening.listener_queue,
        settings=opening.settings,
        bounds=opening.bounds,
        reconnect=reconnect,
        project=opening.project,
    )


def _reached_text(product: ProductPackage, engine: EngineInfo) -> str:
    """The line logged once `connect()` reached `engine`."""
    version = f"{engine.product} {engine.version}"
    match engine.kind:
        case "attached":
            return (
                f"{product.name} connected to an Attached engine, {version}, "
                f"started from {engine.engine_path or 'its executable'}"
            )
        case "daemon":
            return (
                f"{product.name} connected to the Daemon at "
                f"{engine.url or 'its endpoint'}, {version}"
            )
        case "remote":
            return (
                f"{product.name} connected to the remote Daemon at "
                f"{engine.url or 'its address'}, {version}"
            )


async def open_over(
    product: ProductPackage,
    transport: ByteStream,
    *,
    close_timeout: float = CLOSE_TIMEOUT,
    max_listener_queue: int = MAX_LISTENER_QUEUE,
    settings: Mapping[str, object] | None = None,
) -> ClientConnection:
    """Finishes `hello`, naming `product`, over `transport`.

    Such as the Scripted engine's in-memory player (ADR 0069): `connect()`'s
    Connection with no process behind it. `max_listener_queue` and
    `settings` are `connect()`'s, and a bad bound raises `setting_invalid`
    before `hello`.
    """
    listener_queue = listener_queue_of(max_listener_queue)
    bounds = bounds_of(settings)
    return await ClientConnection.open(
        transport,
        needs=product.needs,
        error_classes=product.error_classes,
        close_timeout=close_timeout,
        listener_queue=listener_queue,
        settings=settings,
        bounds=bounds,
    )
