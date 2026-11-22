"""Whether a Client's executable would be its major's pick once it ran (ADR 0098).

Read from the list of installed executables in a real Goodfetch home: the
decision `installed::pick` makes, which a refusal's `fix` names Foxhound
by. TypeScript's `installed-executables.test.ts` is its twin.
"""

import json
import os
from collections.abc import Callable
from datetime import UTC, datetime
from pathlib import Path

import pytest
from goodfetch_collie_client._installed_executables import (
    PickedClient,
    would_be_the_pick,
)
from goodfetch_collie_client.generated.models import Product

NOW = datetime(2026, 11, 21, 12, 0, 0, tzinfo=UTC)
SEEN_LATELY = "2026-10-22T12:00:01Z"
"""Seen 29 days, 23 hours, 59 minutes and 59 seconds before `NOW`: it still counts."""
SEEN_30_DAYS_AGO = "2026-10-22T12:00:00Z"
"""Seen 30 days before `NOW`, the default `browser_cache.max_unused_age`: no longer."""
MODIFIED_AT = "2026-01-02T08:00:00Z"
MODIFIED_SECONDS = int(datetime(2026, 1, 2, 8, 0, 0, tzinfo=UTC).timestamp())
EXECUTABLE_BYTES = b"foxhound"
COPY = {
    "kind": "executable_copy",
    "name": "goodfetch-foxhound",
    "build": "0.4.0-0123456789abcdef",
}


@pytest.fixture
def home(tmp_path: Path) -> Path:
    """A Goodfetch home with a Foxhound executable in `bin`, at `MODIFIED_AT`."""
    (tmp_path / "bin").mkdir()
    executable = tmp_path / "bin" / "goodfetch-foxhound"
    executable.write_bytes(EXECUTABLE_BYTES)
    _modified(executable, MODIFIED_SECONDS * 10**9)
    return tmp_path


def _modified(path: Path, nanoseconds: int) -> None:
    os.utime(path, ns=(nanoseconds, nanoseconds))


def entry(home: Path, **fields: object) -> dict[str, object]:
    """A Foxhound executable's entry at 0.4.0, at its path as it is, seen lately."""
    return {
        "kind": "product_executable",
        "product": "foxhound",
        "product_version": "0.4.0",
        "path": str(home / "bin" / "goodfetch-foxhound"),
        "size": len(EXECUTABLE_BYTES),
        "modified_at": MODIFIED_AT,
        "pins": [],
        "last_seen": SEEN_LATELY,
        **fields,
    }


def gone(home: Path, last_seen: str, copy: object) -> dict[str, object]:
    """An entry whose executable is gone from its path, at `last_seen`, with `copy`."""
    return entry(
        home, path=str(home / "bin" / "removed"), last_seen=last_seen, copy=copy
    )


def listed(
    home: Path, entries: list[dict[str, object]], format_version: object = "0.1"
) -> None:
    """Writes `home`'s list of installed executables."""
    document = {"format_version": format_version, "executables": entries}
    (home / "installed-executables.json").write_text(json.dumps(document))


def copy_in(root: Path) -> None:
    """Puts the versioned copy `COPY` in the Browser cache at `root`."""
    folder = root / COPY["kind"] / COPY["name"] / COPY["build"]
    (folder / "files").mkdir(parents=True)
    document = {"format_version": "0.1", "artifact": COPY}
    (folder / "entry.json").write_text(json.dumps(document))


def collie(version: str) -> PickedClient:
    """A Collie Client at `version`."""
    return PickedClient(Product.collie, version)


@pytest.mark.covers("story:11-293")
def test_is_the_pick_with_no_list_of_installed_executables(home: Path) -> None:
    """No list names no executable."""
    assert would_be_the_pick(collie("0.4.0"), str(home), {}, NOW)


@pytest.mark.covers("story:11-293")
def test_is_not_the_pick_while_a_foxhound_of_the_line_is_at_its_path(
    home: Path,
) -> None:
    """Foxhound outranks Collie at one version."""
    listed(home, [entry(home)])

    assert not would_be_the_pick(collie("0.4.0"), str(home), {}, NOW)


@pytest.mark.covers("story:11-293")
def test_is_not_the_pick_while_an_older_foxhound_of_the_line_is_at_its_path(
    home: Path,
) -> None:
    """Foxhound before Collie, then the newer version."""
    listed(home, [entry(home)])

    assert not would_be_the_pick(collie("0.4.1"), str(home), {}, NOW)


@pytest.mark.covers("story:11-293")
def test_reads_a_file_time_with_a_fraction_of_a_second_cut_to_whole_seconds(
    home: Path,
) -> None:
    """A file system with finer times reads as the engine records it."""
    _modified(
        home / "bin" / "goodfetch-foxhound", MODIFIED_SECONDS * 10**9 + 750_000_000
    )
    listed(home, [entry(home, last_seen=SEEN_30_DAYS_AGO)])

    assert not would_be_the_pick(collie("0.4.0"), str(home), {}, NOW)


@pytest.mark.covers("story:11-293")
def test_reads_a_time_at_another_offset_as_the_same_point_in_time(home: Path) -> None:
    """`+01:00` an hour later is `SEEN_LATELY`."""
    listed(home, [gone(home, "2026-10-22T13:00:01+01:00", COPY)])
    copy_in(home / "browsers")

    assert not would_be_the_pick(collie("0.4.0"), str(home), {}, NOW)


@pytest.mark.covers("story:11-293")
def test_is_not_the_pick_while_a_foxhound_gone_from_its_path_has_its_copy(
    home: Path,
) -> None:
    """Seen lately, its versioned copy in the Browser cache can start it."""
    listed(home, [gone(home, SEEN_LATELY, COPY)])
    copy_in(home / "browsers")

    assert not would_be_the_pick(collie("0.4.0"), str(home), {}, NOW)


@pytest.mark.covers("story:11-293")
def test_reads_the_copy_in_the_browser_cache_goodfetch_browsers_path_names(
    home: Path,
) -> None:
    """The variable's cache, not `browsers` in Goodfetch home."""
    cache = home / "elsewhere"
    listed(home, [gone(home, SEEN_LATELY, COPY)])
    copy_in(cache)

    env = {"GOODFETCH_BROWSERS_PATH": str(cache)}
    assert not would_be_the_pick(collie("0.4.0"), str(home), env, NOW)


@pytest.mark.covers("story:11-293")
def test_is_not_the_pick_while_a_foxhound_seen_in_the_future_has_its_copy(
    home: Path,
) -> None:
    """A time past `now` still counts, as the engine counts it."""
    listed(home, [gone(home, "2026-12-01T00:00:00Z", COPY)])
    copy_in(home / "browsers")

    assert not would_be_the_pick(collie("0.4.0"), str(home), {}, NOW)


CANNOT_BE_IT: dict[str, Callable[[Path], dict[str, object]]] = {
    "copy-not-in-cache": lambda home: gone(home, SEEN_LATELY, COPY),
    "no-copy": lambda home: gone(home, SEEN_LATELY, None),
    "seen-30-days-ago": lambda home: gone(home, SEEN_30_DAYS_AGO, COPY),
    "rewritten-and-seen-30-days-ago": lambda home: entry(
        home, size=9, last_seen=SEEN_30_DAYS_AGO
    ),
    "time-a-second-off-and-seen-30-days-ago": lambda home: entry(
        home, modified_at="2026-01-02T07:59:59Z", last_seen=SEEN_30_DAYS_AGO
    ),
    "time-with-a-fraction-and-seen-30-days-ago": lambda home: entry(
        home, modified_at="2026-01-02T08:00:00.5Z", last_seen=SEEN_30_DAYS_AGO
    ),
    "another-line": lambda home: entry(home, product_version="0.5.0"),
    "linked-program": lambda home: entry(home, kind="linked_program"),
    "unreadable": lambda home: entry(home, product_version="0.4"),
    "size-not-a-number": lambda home: entry(home, size=True),
    "day-that-does-not-exist": lambda home: entry(
        home, last_seen="2026-02-30T00:00:00Z"
    ),
    "more-than-nine-digits-of-fraction": lambda home: entry(
        home, last_seen="2026-10-22T12:00:01.0000000001Z"
    ),
}
"""Foxhound entries that do not count, cannot start or cannot be read."""


@pytest.mark.covers("story:11-293")
@pytest.mark.parametrize("case", list(CANNOT_BE_IT))
def test_is_the_pick_while_a_foxhound_is_installed_but_cannot_be_it(
    home: Path, case: str
) -> None:
    """An entry that does not count, cannot start or cannot be read is no pick."""
    listed(home, [CANNOT_BE_IT[case](home)])
    copy_in(home / "unused")

    assert would_be_the_pick(collie("0.4.0"), str(home), {}, NOW)


def _list_bytes(home: Path, before: bytes = b"", after: bytes = b"") -> bytes:
    document = {"format_version": "0.1", "executables": [entry(home)]}
    text = json.dumps(document).encode()
    return before + text[:-1] + after + b"}"


def _write_list(home: Path, before: bytes = b"", after: bytes = b"") -> None:
    (home / "installed-executables.json").write_bytes(_list_bytes(home, before, after))


UNREADABLE_LISTS: dict[str, Callable[[Path], object]] = {
    "another-major": lambda home: listed(home, [entry(home)], "1.0"),
    "no-format-version": lambda home: listed(home, [entry(home)], None),
    "not-json": lambda home: (home / "installed-executables.json").write_text("{"),
    "not-utf-8": lambda home: _write_list(home, after=b',"note":"\xff"'),
    "byte-order-mark": lambda home: _write_list(home, before=b"\xef\xbb\xbf"),
    "not-a-number-constant": lambda home: _write_list(home, after=b',"note":NaN'),
    "nested-deeper-than-its-reader-goes": lambda home: _write_list(
        home, after=b',"note":' + b"[" * 100_000 + b"]" * 100_000
    ),
    "format-minor-past-its-bound": lambda home: listed(
        home, [entry(home)], "0." + "1" * 5000
    ),
}
"""Writers of a list the engine would not read."""


@pytest.mark.covers("story:11-293")
@pytest.mark.parametrize("case", list(UNREADABLE_LISTS))
def test_is_the_pick_with_a_list_it_cannot_read(home: Path, case: str) -> None:
    """A list the engine would not read names no executable."""
    UNREADABLE_LISTS[case](home)

    assert would_be_the_pick(collie("0.4.0"), str(home), {}, NOW)


@pytest.mark.covers("story:11-293")
@pytest.mark.parametrize(
    ("own", "installed", "pick"),
    [
        pytest.param("1.0.0-rc.2", "1.0.1", False, id="a-newer-patch"),
        pytest.param("1.0.9", "1.0.10", False, id="patch-10-above-patch-9"),
        pytest.param("1.0.9", "1.0." + "1" * 5000, False, id="a-patch-of-5000-digits"),
        pytest.param(
            "1.0.0-" + "9" * 20,
            "1.0.0-1" + "0" * 20,
            True,
            id="pre-release-numbers-past-64-bits-as-words",
        ),
        pytest.param(
            "1.0.0-rc.10",
            "1.0.0-rc." + "0" * 30 + "11",
            False,
            id="pre-release-number-with-leading-zeros",
        ),
        pytest.param("1.2.0", "1.10.0", False, id="a-newer-minor-of-the-major"),
        pytest.param("1.0.0-rc.2", "1.0.0", False, id="release-above-its-pre-release"),
        pytest.param("1.0.0-rc.2", "1.0.0-rc.10", False, id="pre-release-numbers"),
        pytest.param("1.0.0-1", "1.0.0-alpha", False, id="a-word-above-a-number"),
        pytest.param("1.0.0-rc", "1.0.0-rc.1", False, id="a-longer-pre-release"),
        pytest.param("1.0.0-rc.2", "1.0.0-rc.1", True, id="an-older-pre-release"),
        pytest.param("1.4.0", "1.3.9", True, id="an-older-release"),
        pytest.param("1.4.0", "1.4.0", True, id="its-own-version"),
    ],
)
def test_with_a_collie_installed_is_the_pick_unless_it_is_newer(
    home: Path, own: str, installed: str, pick: bool
) -> None:
    """Semantic Versioning's precedence, as the engine orders Product versions."""
    listed(home, [entry(home, product="collie", product_version=installed)])

    assert would_be_the_pick(collie(own), str(home), {}, NOW) is pick


@pytest.mark.covers("story:11-293")
@pytest.mark.parametrize(
    ("installed", "pick"),
    [
        pytest.param("0.4.1", False, id="a-newer-foxhound"),
        pytest.param("0.4.0", True, id="its-own-version"),
    ],
)
def test_a_foxhound_client_is_the_pick_unless_a_newer_foxhound_is_installed(
    home: Path, installed: str, pick: bool
) -> None:
    """Only a newer Foxhound of the line outranks a Foxhound Client."""
    listed(home, [entry(home, product_version=installed)])

    client = PickedClient(Product.foxhound, "0.4.0")
    assert would_be_the_pick(client, str(home), {}, NOW) is pick
