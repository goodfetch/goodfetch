"""The environment map's strict reading at `connect()` (ADR 0083).

As the engine reads its own: the `GOODFETCH_` prefix is reserved, so a
variable off the closed list of deployment variables is `setting_unknown`,
naming the closest one, and an empty value or one that is no Unicode
`setting_invalid`. TypeScript's `deployment-variables.ts` is its twin.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from goodfetch_collie_client.generated.connection import (
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
        SettingInvalidError: A variable is empty, or its value is no Unicode.
    """
    for name in sorted(env):
        if not name.startswith(DEPLOYMENT_PREFIX):
            continue
        if name not in DEPLOYMENT_VARIABLES:
            raise _unknown_variable(name)
        reason = _invalid_reason(env[name])
        if reason is not None:
            raise SettingInvalidError(
                f"`{name}` is not valid in the environment: {reason}",
                fix=f"Set {name} to a non-empty Unicode value, or unset it.",
                details={"key": name, "reason": reason},
            )


def _invalid_reason(value: str) -> str | None:
    """Why `value` cannot be a deployment variable's, or `None`."""
    if not value:
        return "it is empty; unset it instead"
    try:
        value.encode()
    except UnicodeEncodeError:
        # A byte `os.environ` could not decode reads as a lone surrogate.
        return "it is not Unicode"
    return None


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
