"""The project file `connect()` reads (ADR 0083).

The `goodfetch.toml` found in the working directory or the nearest folder
above it, the one the program names, or none, read once at `connect()`
and refused there before any engine is reached, as the engine reads a
project file: its `version` under ADR 0030's default rule, its
`[settings]` table, and nothing else. The file is carried by its absolute
path, so the engine, or a later read of the Run it starts, finds the same
file. TypeScript's `project-file.ts` is its twin.
"""

from __future__ import annotations

import json
import math
import os
import re
import sys
import tomllib
from dataclasses import dataclass
from datetime import date, datetime, time
from pathlib import Path
from typing import TYPE_CHECKING, Literal, cast

from goodfetch_collie_client.generated.connection import (
    PROJECT_FILE,
    PROJECT_FORMAT,
    PROJECT_FORMAT_VERSION,
)
from goodfetch_collie_client.generated.errors import (
    FormatVersionUnsupportedError,
    SettingInvalidError,
)
from goodfetch_collie_client.generated.models import (
    ProjectSettings,
    SettingLimit,
    SettingsLayer,
)
from goodfetch_collie_client.handle import NotGiven

if TYPE_CHECKING:
    from collections.abc import Callable, Mapping

__all__ = ["ConnectTimeout", "ProjectFile", "ProjectTimeout", "read_project"]

_LAYER = SettingsLayer.project_configuration
"""The layer a project file's values and refusals are at."""

_TIMEOUT_CONNECT = "timeout.connect"
"""The key that sets how long opening a Connection waits for `hello`'s answer."""

_LOWEST_MINOR = "lowest_minor"
"""A file's top-level key naming the lowest minor that can use it (ADR 0098)."""

_SETTINGS = "settings"
"""The file's one table."""

_PROJECT_FIX = (
    "name another project file with `project`, or read none with `project=None`"
)
"""What a program does in place of the project file refused."""

_INTEGER_RANGE = range(-(2**63), 2**63)
"""The integers the engine's TOML reads; a file holding another does not read."""

_VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
"""A version `major.minor`, each part canonical."""


@dataclass(frozen=True)
class ProjectTimeout:
    """The `timeout.connect` a project file sets, already checked."""

    value: int
    """Its milliseconds, 1 or more."""


ConnectTimeout = int | ProjectTimeout | NotGiven | None
"""How long opening a Connection waits for `hello`'s answer, already checked.

Milliseconds given as `connect()`'s argument, the project file's
`timeout.connect`, `timeout.connect`'s default when not given, or none for
`None`.
"""


@dataclass(frozen=True)
class ProjectFile:
    """The project file a Connection read: where it is, its settings, its timeout."""

    carried: ProjectSettings
    """Its absolute path and settings, as `run.start` and `config.explain` take them."""
    connect_timeout: ProjectTimeout | None
    """The `timeout.connect` it sets; `None` when it sets none."""


def read_project(
    project: str | os.PathLike[str] | NotGiven | None,
    working_directory: Callable[[], str],
) -> ProjectFile | None:
    """The project file `project` chooses, read.

    The one found upward from `working_directory()` when not given, the
    file it names, a relative one from the working directory, or none for
    `None`.

    Raises:
        SettingInvalidError: At `project_configuration`, when the working
            directory cannot be read, for a named file that is not there,
            for a file that does not read as a project file, and for a
            `timeout.connect` that is no whole number of 1 ms or more.
        FormatVersionUnsupportedError: The file is of a format version this
            Goodfetch cannot read.
    """
    if project is None:
        return None
    try:
        folder = working_directory()
    except OSError as error:
        raise _no_working_directory(error) from error
    file = (
        _find_project(folder)
        if isinstance(project, NotGiven)
        else _absolute(folder, os.fspath(project))
    )
    if file is None:
        return None
    settings = _project_settings(file, _read(file))
    carried = ProjectSettings(path=file, settings=_json_table(file, settings))
    return ProjectFile(
        carried, _connect_timeout_of(file, _nested(settings, _TIMEOUT_CONNECT))
    )


def _absolute(folder: str, project: str) -> str:
    """`project` made absolute from `folder`, as `std::path::absolute` makes it.

    So that every SDK reports the same path: on POSIX `.` components and
    repeated separators dropped and each `..` kept for the file system to
    resolve past a symbolic link; on Windows resolved whole, as
    `GetFullPathNameW` resolves it there.
    """
    if sys.platform == "win32":
        return os.path.abspath(os.path.join(folder, project))
    joined = project if project.startswith("/") else f"{folder}/{project}"
    parts = [part for part in joined.split("/") if part not in {"", "."}]
    return "/" + "/".join(parts)


def _find_project(folder: str) -> str | None:
    """The project file in `folder` or the nearest folder above it."""
    current = Path(folder)
    for each in (current, *current.parents):
        file = each / PROJECT_FILE
        if file.is_file():
            return str(file)
    return None


def _read(file: str) -> str:
    """The text of the project file at `file`."""
    try:
        return Path(file).read_text("utf-8")
    except FileNotFoundError as error:
        reason = "the file is not there"
        raise SettingInvalidError(
            f"The project file {file} cannot be read: {reason}.",
            fix=f"Create it, or {_PROJECT_FIX}.",
            details={"key": file, "layer": _LAYER, "reason": reason},
        ) from error
    except (OSError, UnicodeDecodeError) as error:
        raise _invalid_file(file, str(error)) from error


def _project_settings(file: str, text: str) -> dict[str, object]:
    """The `[settings]` of the project file at `file`, whose text is `text`.

    Once its `version` is checked and any other top-level entry refused, as
    the engine's own read does.
    """
    try:
        parsed = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        raise _invalid_file(file, str(error)) from error
    others = dict(parsed)
    version = others.pop("version", None)
    lowest_minor = others.pop(_LOWEST_MINOR, None)
    settings = others.pop(_SETTINGS, {})
    if not isinstance(version, str):
        raise _invalid_file(file, "it names no `version`")
    found = _version_of(version)
    if found is None:
        raise _invalid_file(file, f"`{version}` is no `major.minor` version")
    if lowest_minor is not None and not _is_minor(lowest_minor):
        raise _invalid_file(file, f"`{_LOWEST_MINOR}` is no minor")
    supported = _version_of(PROJECT_FORMAT_VERSION)
    if supported is None:
        message = "invariant: the generated project format version is `major.minor`"
        raise TypeError(message)
    compatibility = _compatibility_of(supported, found)
    if compatibility != "compatible":
        raise _format_refused(file, version, compatibility)
    if not isinstance(settings, dict):
        raise _invalid_file(file, f"`{_SETTINGS}` is not a table")
    other = next(iter(others), None)
    if other is not None:
        raise _invalid_file(
            file, f"`{other}` is no part of this file; settings go in `[settings]`"
        )
    # A TOML table's keys are strings.
    return cast("dict[str, object]", settings)


_Version = tuple[int, int]
"""A version `major.minor`, as the engine reads one: two numbers of at most 65535."""


def _version_of(text: str) -> _Version | None:
    match = _VERSION.fullmatch(text)
    if match is None:
        return None
    major, minor = int(match[1]), int(match[2])
    return (major, minor) if major <= 0xFFFF and minor <= 0xFFFF else None


def _is_minor(value: object) -> bool:
    return (
        isinstance(value, int) and not isinstance(value, bool) and 0 <= value <= 0xFFFF
    )


_Compatibility = Literal["compatible", "newer_minor", "newer_major", "older_major"]
"""What a reader makes of a file under ADR 0030's default rule."""


def _compatibility_of(reader: _Version, found: _Version) -> _Compatibility:
    """ADR 0030's default rule, for a reader of `reader` and a file of `found`.

    A file of an older line needs `upgrade-files`, a newer line or a newer
    minor of the same line a newer Goodfetch. During `0.y` previews each
    `0.y` is a line of its own, whose only minor is 0.
    """
    reader_line, reader_minor = _line_of(reader)
    found_line, found_minor = _line_of(found)
    if found_line < reader_line:
        return "older_major"
    if found_line > reader_line:
        return "newer_major"
    return "newer_minor" if found_minor > reader_minor else "compatible"


def _line_of(version: _Version) -> tuple[_Version, int]:
    """A version's line and its minor in it."""
    major, minor = version
    return ((0, minor), 0) if major == 0 else ((major, 0), minor)


def _format_refused(
    file: str, found: str, compatibility: _Compatibility
) -> FormatVersionUnsupportedError:
    """The `format_version_unsupported` of the project file at `file`, of `found`."""
    if compatibility == "older_major":
        message = (
            f"this {PROJECT_FORMAT} file is {found}, of an older major; run "
            "`upgrade-files` to upgrade it"
        )
        fix = "Run `upgrade-files`, then try again."
    else:
        message = (
            f"this {PROJECT_FORMAT} file is {found}, newer than this engine reads; "
            "upgrade Goodfetch to read it"
        )
        fix = f"Upgrade Goodfetch to a version that reads {PROJECT_FORMAT} {found}."
    return FormatVersionUnsupportedError(
        f"The project file {file} cannot be read: {message}",
        fix=fix,
        details={
            "format": PROJECT_FORMAT,
            "found": found,
            "supported": PROJECT_FORMAT_VERSION,
            "fix": fix,
        },
    )


def _invalid_file(file: str, reason: str) -> SettingInvalidError:
    """The `setting_invalid` of the project file at `file`, unread for `reason`."""
    return SettingInvalidError(
        f"The project file {file} cannot be read: {reason}.",
        fix=f"Fix the project file at {file}, or {_PROJECT_FIX}.",
        details={"key": file, "layer": _LAYER, "reason": reason},
    )


def _no_working_directory(error: OSError) -> SettingInvalidError:
    """The `setting_invalid` when the working directory cannot be read."""
    reason = f"the working directory cannot be read ({error})"
    return SettingInvalidError(
        f"No project file can be found: {reason}.",
        fix=(
            "Name the project file by its absolute path with `project`, or read "
            "none with `project=None`."
        ),
        details={"key": "project", "layer": _LAYER, "reason": reason},
    )


def _nested(settings: Mapping[str, object], key: str) -> object | None:
    """The value of `key`: written whole as one dotted key, or nested in tables."""
    if key in settings:
        return settings[key]
    value: object | None = settings
    for segment in key.split("."):
        value = _entry(value, segment)
    return value


def _entry(value: object | None, segment: str) -> object | None:
    """The value at `segment` of `value` when it is a table; TOML has no null."""
    if not isinstance(value, dict):
        return None
    # A TOML table's keys are strings.
    return cast("dict[str, object]", value).get(segment)


def _connect_timeout_of(file: str, value: object | None) -> ProjectTimeout | None:
    """The project file's `timeout.connect`, `value`, checked as the engine checks it.

    Raises:
        SettingInvalidError: The value is no whole number of 1 ms or more.
    """
    if value is None:
        return None
    if isinstance(value, int) and not isinstance(value, bool) and value >= 1:
        return ProjectTimeout(value)
    reason = (
        "a timeout of 0 ms would end the opening at once"
        if value == 0 and not isinstance(value, bool)
        else "it is no whole number of milliseconds"
    )
    fix = (
        f"set `{_TIMEOUT_CONNECT}` in the project file to at least 1 ms, or "
        "remove it for 30 s"
    )
    shown = _json_value(file, value)
    raise SettingInvalidError(
        f"The project file's `{_TIMEOUT_CONNECT}`, {json.dumps(shown)}: {reason}; "
        f"{fix}.",
        fix=fix,
        limit=SettingLimit(key=_TIMEOUT_CONNECT, value=shown, layer=_LAYER),
        details={"key": _TIMEOUT_CONNECT, "layer": _LAYER, "reason": reason},
    )


def _json_table(file: str, table: Mapping[str, object]) -> dict[str, object]:
    """The project file at `file`'s TOML table `table`, as JSON.

    An integer as itself, a float JSON cannot hold as `None`, as the engine
    turns them, and a date or time as its ISO 8601 text, which may differ
    from the engine's text of it: Python writes an offset of zero as
    `+00:00`.

    Raises:
        SettingInvalidError: An integer is past what the engine reads.
    """
    return {key: _json_value(file, value) for key, value in table.items()}


def _json_value(file: str, value: object) -> object:
    if isinstance(value, bool | str):
        return value
    if isinstance(value, int):
        if value not in _INTEGER_RANGE:
            raise _invalid_file(
                file, f"the integer `{value}` is past what the engine reads"
            )
        return value
    if isinstance(value, float):
        return value if math.isfinite(value) else None
    if isinstance(value, datetime | date | time):
        return value.isoformat()
    if isinstance(value, list):
        return [_json_value(file, item) for item in cast("list[object]", value)]
    if isinstance(value, dict):
        return _json_table(file, cast("dict[str, object]", value))
    message = f"invariant: TOML reads no value of type {type(value).__name__}"
    raise TypeError(message)
