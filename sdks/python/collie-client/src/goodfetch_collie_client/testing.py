"""The testing kit's shared part (ADRs 0069, 0095 and 0105).

Each Product's kit re-exports `TestEngineOptions`, what a test engine starts
with, and its test engines are an `AsyncTestEngine` and a `TestEngine`, the
async and sync shapes every Product's test engine has. A Product's kit
starts an Attached engine in a temporary Goodfetch home with a cleared
environment, `egress.offline` and `browser_cache.read_only` on, short
durations and no project file unless the test names one; every host its
`network.resolve` maps to loopback alone is the only one a Run's requests
reach, live. Importing it reads, patches and logs nothing (ADR 0078).
Rust's `testing` module of `goodfetch-collie-client` is its twin.
"""

from __future__ import annotations

import ipaddress
import os
import shutil
import tempfile
from dataclasses import dataclass
from datetime import timedelta
from typing import TYPE_CHECKING, Protocol, TypedDict, TypeVar

from goodfetch_collie_client._installed_executables import BROWSERS_PATH_VARIABLE
from goodfetch_collie_client.engine_choice import HOME_VARIABLE, home_of

if TYPE_CHECKING:
    from collections.abc import Mapping
    from typing import Self

    from goodfetch_collie_client.generated.settings import (
        ConnectionBrowserCacheSettings,
        ConnectionEgressSettings,
        ConnectionTimeoutSettings,
        Settings,
    )

__all__ = ["AsyncTestEngine", "TestEngine", "TestEngineOptions"]

_Connection_co = TypeVar("_Connection_co", covariant=True)

_SHORT = timedelta(seconds=5)
"""How long the test engine's calls wait where the catalog's defaults wait 30 s.

A page's actions, navigations, waits and first bytes, and a Run's close, so
a test that went wrong fails in seconds. A test's own settings take the
place of each.
"""


class TestEngineOptions(TypedDict, total=False):
    """What a test engine starts with, each left out for the kit's isolations alone.

    `settings` are the connection's, over the kit's: `network.resolve`'s
    hosts mapped to loopback alone are the hosts its Runs reach, live, and a
    key it gives, such as `timeout.navigation`, takes the place of the
    kit's. `env` is the environment map the kit reads `GOODFETCH_HOME`,
    `GOODFETCH_BROWSERS_PATH` and its Product's engine-path variable from,
    and no other variable, in place of the process's own; the engine's own
    environment stays cleared. `project` is the project file to read (ADR
    0083), a relative one from the working directory, in place of none.
    """

    settings: Settings
    env: Mapping[str, str]
    project: str | os.PathLike[str]


class AsyncTestEngine(Protocol[_Connection_co]):
    """A Product's test engine in the async API, whose connection is the Product's.

    Closing it closes the connection, then removes its temporary Goodfetch
    home; `async with` closes it.
    """

    @property
    def connection(self) -> _Connection_co:
        """The Product's connection, on which the code under test runs unchanged."""
        ...

    async def close(self) -> None:
        """Closes the connection, then removes the temporary Goodfetch home.

        The home is removed even when closing the connection raises.
        """
        ...

    async def __aenter__(self) -> Self:
        """The test engine, for an `async with` block that closes it."""
        ...

    async def __aexit__(self, *_: object) -> None:
        """Closes the test engine."""
        ...


class TestEngine(Protocol[_Connection_co]):
    """A Product's test engine in the sync API, as `AsyncTestEngine` is in async.

    `with` closes it.
    """

    @property
    def connection(self) -> _Connection_co:
        """The Product's connection, on which the code under test runs unchanged."""
        ...

    def close(self) -> None:
        """Closes the connection, then removes the temporary Goodfetch home.

        The home is removed even when closing the connection raises.
        """
        ...

    def __enter__(self) -> Self:
        """The test engine, for a `with` block that closes it."""
        ...

    def __exit__(self, *_: object) -> None:
        """Closes the test engine."""
        ...


@dataclass(frozen=True)
class TestEngineParts:
    """What a Product's kit connects its test engine with, which `parts_of` answers."""

    env: dict[str, str]
    """The engine's environment map, cleared but for its home, browsers and engine."""
    settings: Settings
    """The connection's settings, the test's over the kit's."""
    project: str | os.PathLike[str] | None
    """The project file the test named, else `None`, which reads none."""


def parts_of(
    options: TestEngineOptions, home: str, engine_path_variable: str
) -> TestEngineParts:
    """What a Product's kit connects with in the temporary Goodfetch home `home`.

    The caller's variables come from `options`' `env`, else the process's
    environment, read here; `engine_path_variable` is the Product's.
    """
    caller = options.get("env", os.environ)
    return TestEngineParts(
        env=engine_environment(caller, home, engine_path_variable),
        settings=isolated(options.get("settings", {})),
        project=options.get("project"),
    )


def make_home() -> str:
    """A new temporary Goodfetch home, which `remove_home` removes; it blocks."""
    return tempfile.mkdtemp(prefix="goodfetch-test-engine-")


def remove_home(home: str) -> None:
    """Removes the temporary Goodfetch home `home` and all it holds; it blocks."""
    shutil.rmtree(home, ignore_errors=True)


def engine_environment(
    caller: Mapping[str, str], home: str, engine_path_variable: str
) -> dict[str, str]:
    """The engine's environment map, every other variable of `caller` left out.

    `GOODFETCH_HOME` names `home`, `GOODFETCH_BROWSERS_PATH` the Browser cache
    of `caller`, its own variable's or its Goodfetch home's, and
    `engine_path_variable` is as `caller` sets it.
    """
    environment = {HOME_VARIABLE: home}
    caller_home = home_of(caller)
    cache = caller.get(BROWSERS_PATH_VARIABLE) or (
        None if caller_home is None else os.path.join(caller_home, "browsers")
    )
    if cache is not None:
        environment[BROWSERS_PATH_VARIABLE] = cache
    engine = caller.get(engine_path_variable)
    if engine:
        environment[engine_path_variable] = engine
    return environment


def isolated(settings: Settings) -> Settings:
    """`settings` over the kit's isolations, `settings` itself left unchanged.

    `egress.offline` on, the hosts `network.resolve` maps to loopback alone
    as the only hosts allowed and the private hosts a page may reach,
    `browser_cache.read_only` on, so the caller's Browser cache is left as
    the test found it (issue 3458), and the short durations.
    """
    loopback = _loopback_hosts(settings.get("network", {}).get("resolve"))
    browser_cache: ConnectionBrowserCacheSettings = {
        "read_only": True,
        **settings.get("browser_cache", {}),
    }
    egress: ConnectionEgressSettings = {
        "offline": True,
        "allowed_domains": loopback,
        "private_ranges": list(loopback),
        **settings.get("egress", {}),
    }
    timeout: ConnectionTimeoutSettings = {
        "action": _SHORT,
        "navigation": _SHORT,
        "wait": _SHORT,
        "first_byte": _SHORT,
        "close": _SHORT,
        **settings.get("timeout", {}),
    }
    return {
        **settings,
        "browser_cache": browser_cache,
        "egress": egress,
        "timeout": timeout,
    }


def _loopback_hosts(resolve: Mapping[str, list[str]] | None) -> list[str]:
    """The hosts `resolve` maps to addresses that are all loopback, sorted."""
    if resolve is None:
        return []
    return sorted(
        host
        for host, addresses in resolve.items()
        if addresses and all(_is_loopback(address) for address in addresses)
    )


def _is_loopback(address: str) -> bool:
    """Whether `address` is a loopback IP address, an IPv4-mapped one included."""
    try:
        parsed = ipaddress.ip_address(address)
    except ValueError:
        return False
    if isinstance(parsed, ipaddress.IPv6Address) and parsed.ipv4_mapped is not None:
        return parsed.ipv4_mapped.is_loopback
    return parsed.is_loopback
