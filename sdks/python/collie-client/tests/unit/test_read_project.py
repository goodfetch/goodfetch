"""The project file `connect()` reads (ADR 0083; story 11-75).

It is read as the engine reads one: found upward from the working
directory, named by `project` from it, or none for `None`; refused before
any engine is reached when it is not there, does not read as a project
file, is of a format version this Goodfetch cannot read, or sets a
`timeout.connect` that is no whole number of 1 ms or more. TypeScript's
`project-file.test.ts` is its twin.
"""

from __future__ import annotations

import sys
import tomllib
from typing import TYPE_CHECKING

import pytest
from goodfetch_collie_client._project_file import (
    ProjectFile,
    ProjectTimeout,
    read_project,
)
from goodfetch_collie_client.generated.errors import (
    FormatVersionUnsupportedError,
    SettingInvalidError,
)
from goodfetch_collie_client.generated.models import ProjectSettings
from goodfetch_collie_client.handle import not_given

if TYPE_CHECKING:
    from pathlib import Path


def write(parent: Path, name: str, text: str) -> Path:
    """Writes `text` as `name` in `parent`, creating it, and returns the file."""
    parent.mkdir(parents=True, exist_ok=True)
    file = parent / name
    file.write_text(text, "utf-8")
    return file


def navigation(value: int) -> str:
    """A project file's text that sets `timeout.navigation` to `value` ms."""
    return f'version = "0.1"\n\n[settings.timeout]\nnavigation = {value}\n'


@pytest.fixture
def folder(tmp_path: Path) -> Path:
    """The test's folder, as the OS names it."""
    return tmp_path.resolve()


@pytest.mark.covers("story:11-75")
def test_finds_the_project_file_in_the_nearest_folder_above_the_working_directory(
    folder: Path,
) -> None:
    """The nearest `goodfetch.toml` wins over one further up."""
    file = write(folder / "project", "goodfetch.toml", navigation(45_000))
    deeper = folder / "project" / "src" / "deeper"
    deeper.mkdir(parents=True)
    write(folder, "goodfetch.toml", navigation(60_000))

    assert read_project(not_given, lambda: str(deeper)) == ProjectFile(
        ProjectSettings(path=str(file), settings={"timeout": {"navigation": 45_000}}),
        None,
    )


def other_project(folder: Path) -> tuple[Path, str]:
    """A project file in `folder/work` and the other one `project` names.

    Returns the other file and the working directory, `folder/work`.
    """
    write(folder / "work", "goodfetch.toml", navigation(45_000))
    other = write(folder / "elsewhere", "other.toml", navigation(60_000))
    return other, str(folder / "work")


OTHER_SETTINGS = {"timeout": {"navigation": 60_000}}
"""The settings of the file `other_project` writes for `project` to name."""


@pytest.mark.covers("story:11-75")
def test_reads_the_file_project_names_by_its_absolute_path_and_none_for_none(
    folder: Path,
) -> None:
    """The file named wins over the one the working directory would find."""
    other, work = other_project(folder)

    named = read_project(other, lambda: work)

    assert [
        None if named is None else named.carried,
        read_project(None, lambda: work),
    ] == [ProjectSettings(path=str(other), settings=OTHER_SETTINGS), None]


@pytest.mark.covers("story:11-75")
@pytest.mark.skipif(sys.platform == "win32", reason="a POSIX path")
def test_reads_a_relative_project_from_the_working_directory_keeping_dot_dot(
    folder: Path,
) -> None:
    """It keeps `..` and drops `.`, as Rust's `std::path::absolute` on POSIX."""
    _, work = other_project(folder)

    relative = read_project("../elsewhere/./other.toml", lambda: work)

    assert relative is not None
    assert relative.carried == ProjectSettings(
        path=f"{folder}/work/../elsewhere/other.toml", settings=OTHER_SETTINGS
    )


@pytest.mark.covers("story:11-75")
@pytest.mark.skipif(sys.platform != "win32", reason="a Windows path")
def test_reads_a_relative_project_from_the_working_directory_resolving_dot_dot(
    folder: Path,
) -> None:
    """It resolves `..` and `.`, as Rust's `std::path::absolute` on Windows."""
    other, work = other_project(folder)

    relative = read_project("../elsewhere/./other.toml", lambda: work)

    assert relative is not None
    assert relative.carried == ProjectSettings(path=str(other), settings=OTHER_SETTINGS)


@pytest.mark.covers("story:11-75")
def test_carries_a_date_as_its_text_and_a_dotted_key_unchanged(folder: Path) -> None:
    """A TOML date goes as ISO 8601 text.

    `"timeout.connect"` written whole as one dotted key still bounds `hello`.
    """
    file = write(
        folder,
        "goodfetch.toml",
        'version = "0.1"\n\n[settings]\n"timeout.connect" = 200\nday = 2026-01-02\n',
    )

    assert read_project(file, lambda: str(folder)) == ProjectFile(
        ProjectSettings(
            path=str(file), settings={"timeout.connect": 200, "day": "2026-01-02"}
        ),
        ProjectTimeout(200),
    )


@pytest.mark.covers("story:11-75")
def test_carries_a_float_json_cannot_hold_as_null(folder: Path) -> None:
    """`inf` and `nan` go as null, as the engine turns them."""
    file = write(
        folder,
        "goodfetch.toml",
        'version = "0.1"\n\n[settings]\nlow = -inf\nnone = nan\n',
    )

    project = read_project(file, lambda: str(folder))

    assert project is not None
    assert project.carried.settings == {"low": None, "none": None}


@pytest.mark.covers("story:11-75")
def test_refuses_a_working_directory_that_cannot_be_read() -> None:
    """`setting_invalid` names `project`, since no file can be found."""

    def removed() -> str:
        raise FileNotFoundError(2, "No such file or directory")

    with pytest.raises(SettingInvalidError) as refused:
        read_project(not_given, removed)

    assert (refused.value.details["key"], refused.value.details["layer"]) == (
        "project",
        "project_configuration",
    )


FIX_ELSE = "name another project file with `project`, or read none with `project=None`"
"""How each refused project file's `fix` ends."""


@pytest.mark.covers("story:11-75")
def test_refuses_a_named_file_that_is_not_there_with_setting_invalid(
    folder: Path,
) -> None:
    """At `project_configuration`, its `fix` to create it or to read none."""
    file = folder / "goodfetch.toml"

    with pytest.raises(SettingInvalidError) as refused:
        read_project(file, lambda: str(folder))

    details = refused.value.details
    assert (refused.value.fix, details["key"], details["layer"], details["reason"]) == (
        f"Create it, or {FIX_ELSE}.",
        str(file),
        "project_configuration",
        "the file is not there",
    )


@pytest.mark.covers("story:11-75")
def test_refuses_a_file_that_is_no_toml_with_setting_invalid(folder: Path) -> None:
    """At `project_configuration`, caused by the TOML reader's own error."""
    file = write(folder, "goodfetch.toml", 'version = "0.1"\n[settings\n')

    with pytest.raises(SettingInvalidError) as refused:
        read_project(file, lambda: str(folder))

    details = refused.value.details
    assert (refused.value.fix, details["key"], details["layer"]) == (
        f"Fix the project file at {file}, or {FIX_ELSE}.",
        str(file),
        "project_configuration",
    )
    assert isinstance(refused.value.__cause__, tomllib.TOMLDecodeError)


@pytest.mark.covers("story:11-75")
@pytest.mark.parametrize(
    ("text", "reason"),
    [
        pytest.param(
            "[settings.timeout]\nnavigation = 45000\n",
            "it names no `version`",
            id="no-version",
        ),
        pytest.param(
            'version = "0.01"\n',
            "`0.01` is no `major.minor` version",
            id="no-major-minor",
        ),
        pytest.param(
            'version = "0.1"\nlowest_minor = -1\n',
            "`lowest_minor` is no minor",
            id="lowest-minor-negative",
        ),
        pytest.param(
            'version = "0.1"\nlowest_minor = 1.0\n',
            "`lowest_minor` is no minor",
            id="lowest-minor-float",
        ),
        pytest.param(
            'version = "0.1"\nlowest_minor = true\n',
            "`lowest_minor` is no minor",
            id="lowest-minor-bool",
        ),
        pytest.param(
            'version = "0.1"\n\n[settings]\nseed = 9223372036854775808\n',
            "the integer `9223372036854775808` is past what the engine reads",
            id="integer-past-i64",
        ),
        pytest.param(
            'version = "0.1"\nsettings = 3\n',
            "`settings` is not a table",
            id="settings-no-table",
        ),
        pytest.param(
            'version = "0.1"\ntimeout = 3\n',
            "`timeout` is no part of this file; settings go in `[settings]`",
            id="other-top-level-key",
        ),
    ],
)
def test_refuses_a_file_that_does_not_read_with_setting_invalid(
    folder: Path, text: str, reason: str
) -> None:
    """At `project_configuration`, its `fix` to fix it or to read none."""
    file = write(folder, "goodfetch.toml", text)

    with pytest.raises(SettingInvalidError) as refused:
        read_project(file, lambda: str(folder))

    details = refused.value.details
    assert (refused.value.fix, details["key"], details["layer"], details["reason"]) == (
        f"Fix the project file at {file}, or {FIX_ELSE}.",
        str(file),
        "project_configuration",
        reason,
    )


@pytest.mark.covers("story:11-75")
@pytest.mark.parametrize(
    ("version", "fix"),
    [
        pytest.param(
            "9.0",
            "Upgrade Goodfetch to a version that reads user-configuration 9.0.",
            id="newer-major",
        ),
        pytest.param(
            "0.2",
            "Upgrade Goodfetch to a version that reads user-configuration 0.2.",
            id="newer-preview-line",
        ),
        pytest.param(
            "0.0", "Run `upgrade-files`, then try again.", id="older-preview-line"
        ),
    ],
)
def test_refuses_a_file_of_another_format_version(
    folder: Path, version: str, fix: str
) -> None:
    """With `format_version_unsupported`, under ADR 0030's rule."""
    file = write(folder, "goodfetch.toml", f'version = "{version}"\n')

    with pytest.raises(FormatVersionUnsupportedError) as refused:
        read_project(file, lambda: str(folder))

    details = refused.value.details
    assert (refused.value.fix, details["format"], details["found"]) == (
        fix,
        "user-configuration",
        version,
    )
    assert details["supported"] == "0.1"


@pytest.mark.covers("story:11-75", "setting:timeout.connect")
@pytest.mark.parametrize(
    ("value", "reason"),
    [
        pytest.param("0", "a timeout of 0 ms would end the opening at once", id="0"),
        pytest.param('"30s"', "it is no whole number of milliseconds", id="text"),
        pytest.param("1.5", "it is no whole number of milliseconds", id="fraction"),
        pytest.param(
            "200.0", "it is no whole number of milliseconds", id="whole-float"
        ),
        pytest.param("true", "it is no whole number of milliseconds", id="bool"),
    ],
)
def test_refuses_a_timeout_connect_that_is_no_whole_number_of_1_ms_or_more(
    folder: Path, value: str, reason: str
) -> None:
    """`setting_invalid` names the key and the project file's layer."""
    file = write(
        folder,
        "goodfetch.toml",
        f'version = "0.1"\n\n[settings.timeout]\nconnect = {value}\n',
    )

    with pytest.raises(SettingInvalidError) as refused:
        read_project(file, lambda: str(folder))

    limit = refused.value.limit
    assert limit is not None
    assert (limit.key, limit.layer) == ("timeout.connect", "project_configuration")
    details = refused.value.details
    assert (details["key"], details["layer"], details["reason"]) == (
        "timeout.connect",
        "project_configuration",
        reason,
    )
