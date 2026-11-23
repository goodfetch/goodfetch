"""The environment map's strict reading at `connect()` (ADR 0083).

As the engine reads its own: the `GOODFETCH_` prefix is reserved, so a
variable off the closed list of deployment variables is `setting_unknown`,
naming the closest one, and an empty value, one that is no Unicode or one
the engine cannot parse `setting_invalid`. TypeScript's
`deployment-variables.ts` is its twin.
"""

from __future__ import annotations

import ipaddress
import re
from typing import TYPE_CHECKING

from goodfetch_collie_client.generated.connection import (
    CHANNELS,
    DEPLOYMENT_PREFIX,
    DEPLOYMENT_VARIABLES,
)
from goodfetch_collie_client.generated.errors import (
    SettingInvalidError,
    SettingUnknownError,
)

if TYPE_CHECKING:
    from collections.abc import Mapping


def check_deployment_variables(env: Mapping[str, str]) -> None:
    """Checks every `GOODFETCH_` variable of `env` against the closed list.

    The first by name fails.

    Raises:
        SettingUnknownError: A variable is not on the list.
        SettingInvalidError: A variable is empty, its value is no Unicode,
            `GOODFETCH_CHANNEL` names no channel,
            `GOODFETCH_DAEMON_DRAIN_GRACE` is no whole number of milliseconds
            or `GOODFETCH_DAEMON_ADDRESS` or `GOODFETCH_OPS_ADDRESS` is no IP
            address and port.
    """
    for name in sorted(env):
        if not name.startswith(DEPLOYMENT_PREFIX):
            continue
        if name not in DEPLOYMENT_VARIABLES:
            raise _unknown_variable(name)
        reason = _invalid_reason(name, env[name])
        if reason is not None:
            raise SettingInvalidError(
                f"`{name}` is not valid in the environment: {reason}",
                fix=f"Set {name} to {_taken_value(name)}, or unset it.",
                details={"key": name, "reason": reason},
            )


_CHANNEL = "GOODFETCH_CHANNEL"
_DRAIN_GRACE = "GOODFETCH_DAEMON_DRAIN_GRACE"
_DAEMON_ADDRESS = "GOODFETCH_DAEMON_ADDRESS"
_OPS_ADDRESS = "GOODFETCH_OPS_ADDRESS"
_LARGEST_MILLISECONDS = 2**64 - 1
"""The largest drain grace the engine reads, its `u64`'s."""
_MILLISECONDS = re.compile(r"\+?([0-9]+)")
"""A whole number of milliseconds as Rust's `u64::from_str` reads it."""
_SOCKET_ADDRESS = re.compile(
    r"(?:\[(?P<v6>[0-9A-Fa-f:.]+)(?:%(?P<scope>[0-9]+))?\]|(?P<v4>[0-9.]+)):(?P<port>[0-9]+)"
)
"""An address and port in the shape Rust's `SocketAddr::from_str` reads.

An IPv4 address, or an IPv6 one in brackets with an optional decimal
scope, then `:` and a decimal port; `ipaddress` then checks the address.
"""
_LARGEST_PORT = 2**16 - 1
_LARGEST_SCOPE = 2**32 - 1


def _invalid_reason(name: str, value: str) -> str | None:
    """Why `value` cannot be the deployment variable `name`'s, or `None`."""
    if not value:
        return "it is empty; unset it instead"
    try:
        value.encode()
    except UnicodeEncodeError:
        # A byte `os.environ` could not decode reads as a lone surrogate.
        return "it is not Unicode"
    if name == _CHANNEL and value not in CHANNELS:
        return f"`{value}` is no channel; one of {', '.join(CHANNELS)}"
    if name == _DRAIN_GRACE:
        milliseconds = _MILLISECONDS.fullmatch(value)
        if milliseconds is None or not _at_most(milliseconds[1], _LARGEST_MILLISECONDS):
            return f"`{value}` is no whole number of milliseconds"
    if name in (_DAEMON_ADDRESS, _OPS_ADDRESS) and not _is_socket_address(value):
        return f"`{value}` is no `host:port`"
    return None


def _is_socket_address(value: str) -> bool:
    """Whether Rust's `SocketAddr::from_str`, as the engine reads, reads `value`."""
    parts = _SOCKET_ADDRESS.fullmatch(value)
    if parts is None or not _at_most(parts["port"], _LARGEST_PORT):
        return False
    scope = parts["scope"]
    if scope is not None and not _at_most(scope, _LARGEST_SCOPE):
        return False
    try:
        if parts["v4"] is not None:
            ipaddress.IPv4Address(parts["v4"])
        else:
            ipaddress.IPv6Address(parts["v6"])
    except ValueError:
        return False
    return True


def _at_most(digits: str, largest: int) -> bool:
    """Whether the decimal `digits`, leading zeros and all, are at most `largest`.

    Leading zeros go first, so no length of them reaches `int`'s limit on
    the digits it reads.
    """
    significant = digits.lstrip("0")
    return len(significant) <= len(str(largest)) and int(significant or "0") <= largest


def _taken_value(name: str) -> str:
    """What the deployment variable `name` takes, as its `fix` says it."""
    if name == _CHANNEL:
        return f"one of {', '.join(CHANNELS)}"
    if name == _DRAIN_GRACE:
        return "a whole number of milliseconds"
    if name == _DAEMON_ADDRESS:
        return "an IP address and port such as 127.0.0.1:7900"
    if name == _OPS_ADDRESS:
        return "an IP address and port such as 0.0.0.0:9464"
    return "a non-empty Unicode value"


def _unknown_variable(name: str) -> SettingUnknownError:
    closest = _nearest(name)
    if closest is None:
        return SettingUnknownError(
            f"`{name}` in the environment is no deployment variable.",
            fix=(
                f"Unset {name}: the {DEPLOYMENT_PREFIX} prefix is reserved for the "
                f"deployment variables, {', '.join(DEPLOYMENT_VARIABLES)}."
            ),
            details={"key": name},
        )
    return SettingUnknownError(
        f"`{name}` in the environment is no deployment variable; "
        f"did you mean `{closest}`?",
        fix=f"Rename {name} to {closest}, or unset it.",
        details={"key": name, "closest": closest},
    )


def _nearest(name: str) -> str | None:
    """The deployment variable closest to `name`, as the engine finds it.

    One within a third of its length in edits (at least two), the first by
    name among the closest.
    """
    allowed = max(len(name) // 3, 2)
    near = [
        (edits, variable)
        for variable in DEPLOYMENT_VARIABLES
        if (edits := _distance(name, variable)) <= allowed
    ]
    return min(near)[1] if near else None


def _distance(left: str, right: str) -> int:
    """Levenshtein's distance between `left` and `right`, by code points."""
    previous = list(range(len(right) + 1))
    for row, left_point in enumerate(left):
        current = [row + 1]
        for column, right_point in enumerate(right):
            current.append(
                min(
                    previous[column] + (left_point != right_point),
                    current[column] + 1,
                    previous[column + 1] + 1,
                )
            )
        previous = current
    return previous[len(right)]
