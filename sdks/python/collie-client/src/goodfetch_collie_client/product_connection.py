"""What a Product's package builds its Connection type on (ADRs 0024, 0090 and 0095).

It finds the Product's executable, the one its engine-path variable names or
else the one its platform wheel holds, starts it as an Attached engine with
exactly the program's environment map, and finishes `hello`, naming the
Product. The other Product's variable is never read here. TypeScript's
`product-connection.ts` is its twin.
"""

from __future__ import annotations

import os
import sys
from dataclasses import dataclass
from typing import TYPE_CHECKING

from goodfetch_collie_client.attached_engine import AttachedEngine
from goodfetch_collie_client.connection import CLOSE_TIMEOUT, ClientConnection
from goodfetch_collie_client.generated.errors import SettingInvalidError
from goodfetch_collie_client.handle import NotGiven, not_given, timeout_of
from goodfetch_collie_client.limits import limit_list

if TYPE_CHECKING:
    from collections.abc import Mapping
    from datetime import timedelta
    from pathlib import Path

    from anyio.abc import ByteStream

    from goodfetch_collie_client.generated.models import Product
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


async def open_attached(
    product: ProductPackage,
    *,
    env: Mapping[str, str] | None = None,
    close_timeout: float = CLOSE_TIMEOUT,
    limits: Limits | None = None,
    connect_timeout: timedelta | NotGiven | None = not_given,
) -> ClientConnection:
    """Starts `product`'s engine as an Attached engine, then `hello` and `limits`.

    `env` is the environment map, read here and never at import: the
    process's own when left out (ADRs 0062 and 0083). The engine gets
    exactly this map. `connect_timeout` is how long `hello` waits for its answer:
    `timeout.connect`'s 30 s when left out, none for `None`; past it,
    `DeadlineExceededError` naming `timeout.connect` raises. A `limits`
    amount the SDK cannot turn exactly, or a `connect_timeout` of zero or less
    (`setting_invalid`), raises before the engine starts. A missing
    executable (`engine_not_found`), a refused Product (`not_in_product`) or
    version, or a refused Limit raises here, with a `fix`, leaving no engine
    behind.
    """
    connection_limits = limit_list(limits)
    checked = (
        connect_timeout
        if isinstance(connect_timeout, NotGiven)
        else timeout_of(connect_timeout, "timeout.connect")
    )
    environment = dict(os.environ if env is None else env)
    location = locate_engine(environment, product)
    engine = await AttachedEngine.start(
        location.path, environment, location.not_found_fix
    )
    return await ClientConnection.open(
        engine.transport,
        needs=product.needs,
        error_classes=product.error_classes,
        engine=engine,
        close_timeout=close_timeout,
        limits=connection_limits,
        connect_timeout=checked,
    )


async def open_over(
    product: ProductPackage,
    transport: ByteStream,
    *,
    close_timeout: float = CLOSE_TIMEOUT,
) -> ClientConnection:
    """Finishes `hello`, naming `product`, over `transport`.

    Such as the Scripted engine's in-memory player (ADR 0069): `connect()`'s
    Connection with no process behind it.
    """
    return await ClientConnection.open(
        transport,
        needs=product.needs,
        error_classes=product.error_classes,
        close_timeout=close_timeout,
    )
