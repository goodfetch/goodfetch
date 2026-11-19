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
from typing import TYPE_CHECKING

from goodfetch_collie_client.attached_engine import AttachedEngine
from goodfetch_collie_client.connection import (
    CLOSE_TIMEOUT,
    ClientConnection,
    DaemonHello,
    EngineInfo,
    Reached,
)
from goodfetch_collie_client.engine_choice import AttachedReach, reach_engine
from goodfetch_collie_client.generated.errors import SettingInvalidError
from goodfetch_collie_client.handle import NotGiven, not_given, timeout_of
from goodfetch_collie_client.host_logging import CLIENT_LOGGER
from goodfetch_collie_client.limits import limit_list
from goodfetch_collie_client.listeners import MAX_LISTENER_QUEUE, listener_queue_of

if TYPE_CHECKING:
    from collections.abc import Mapping, Sequence
    from datetime import timedelta
    from pathlib import Path

    from anyio.abc import ByteStream

    from goodfetch_collie_client.engine_choice import DaemonReach, EngineChoice
    from goodfetch_collie_client.generated.models import Product, RunLimit
    from goodfetch_collie_client.goodfetch_error import ErrorClass
    from goodfetch_collie_client.limits import Limits


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


@dataclass(frozen=True)
class EngineLocation:
    """Where to start a Product's engine, and what to do when it is not there."""

    path: str
    """The executable."""
    not_found_fix: str
    """The `fix` of `engine_not_found` when `path` is no executable."""


def locate_engine(env: Mapping[str, str], product: ProductPackage) -> EngineLocation:
    """The executable `env` names for `product`, or its platform wheel's.

    Raises:
        SettingInvalidError: The variable is set but empty.
    """
    variable = product.engine_path_variable
    name, executable, package = product.name, product.executable, product.package
    named = env.get(variable)
    if named == "":
        # An empty value is never taken for unset (ADR 0083).
        raise SettingInvalidError(
            f"{variable} is set but empty",
            fix=(
                f"Set {variable} to {name}'s executable, {executable}, or leave it "
                f"unset to use the one {package} installed."
            ),
            details={"key": variable, "reason": "an empty value names no executable"},
        )
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
) -> ClientConnection:
    """Reaches the engine `engine` chooses for `product`, then `hello` and `limits`.

    `engine` is `auto` unless given: the remote Daemon `GOODFETCH_URL` names,
    else the running Daemon of the SDK's line, else an Attached engine (ADR
    0024). `env` is the environment map, read here and never at import: the
    process's own when left out (ADRs 0062 and 0083). An Attached engine,
    and a Daemon this call starts, gets exactly this map. `connect_timeout`
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
    raises here, with a `fix`, leaving no Attached engine behind. The engine
    reached is logged at info on the `goodfetch.client` logger, its
    `EngineInfo` as the record's `engine`.
    """
    connection_limits = limit_list(limits)
    listener_queue = listener_queue_of(max_listener_queue)
    checked = (
        connect_timeout
        if isinstance(connect_timeout, NotGiven)
        else timeout_of(connect_timeout, "timeout.connect")
    )
    environment = dict(os.environ if env is None else env)
    reach = await reach_engine(
        engine, environment, lambda: locate_engine(environment, product)
    )
    if isinstance(reach, AttachedReach):
        connection = await _open_attached(
            product,
            environment,
            reach.home,
            close_timeout,
            connection_limits,
            checked,
            listener_queue,
        )
    else:
        connection = await _open_daemon(
            product,
            reach,
            close_timeout,
            connection_limits,
            checked,
            listener_queue,
        )
    info = connection.engine
    logging.getLogger(CLIENT_LOGGER).info(
        _reached_text(product, info), extra={"engine": info}
    )
    return connection


async def _open_attached(
    product: ProductPackage,
    env: Mapping[str, str],
    home: str | None,
    close_timeout: float,
    limits: Sequence[RunLimit],
    connect_timeout: int | NotGiven | None,
    listener_queue: int,
) -> ClientConnection:
    """Starts `product`'s engine as an Attached engine with `env`, then `hello`."""
    location = locate_engine(env, product)
    engine = await AttachedEngine.start(location.path, env, location.not_found_fix)
    return await ClientConnection.open(
        engine.transport,
        needs=product.needs,
        error_classes=product.error_classes,
        engine=engine,
        reached=Reached("attached", home, None),
        close_timeout=close_timeout,
        limits=limits,
        connect_timeout=connect_timeout,
        listener_queue=listener_queue,
    )


async def _open_daemon(
    product: ProductPackage,
    reach: DaemonReach,
    close_timeout: float,
    limits: Sequence[RunLimit],
    connect_timeout: int | NotGiven | None,
    listener_queue: int,
) -> ClientConnection:
    """Finishes `hello` with a Daemon, its token in it.

    One that closes the Connection before it answers fails with
    `daemon_unreachable`.
    """
    return await ClientConnection.open(
        reach.stream,
        needs=product.needs,
        error_classes=product.error_classes,
        reached=Reached(reach.kind, reach.home, reach.url),
        daemon=DaemonHello(reach.token, reach.closed_at_hello),
        close_timeout=close_timeout,
        limits=limits,
        connect_timeout=connect_timeout,
        listener_queue=listener_queue,
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
) -> ClientConnection:
    """Finishes `hello`, naming `product`, over `transport`.

    Such as the Scripted engine's in-memory player (ADR 0069): `connect()`'s
    Connection with no process behind it. `max_listener_queue` is
    `connect()`'s, and a bad one raises `setting_invalid` before `hello`.
    """
    listener_queue = listener_queue_of(max_listener_queue)
    return await ClientConnection.open(
        transport,
        needs=product.needs,
        error_classes=product.error_classes,
        close_timeout=close_timeout,
        listener_queue=listener_queue,
    )
