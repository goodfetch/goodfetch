"""Whether a Client's executable would be its major's pick once it ran (ADR 0098).

Read from the list of installed executables in Goodfetch home,
`installed-executables.json`, as the engine's `installed::pick` decides it:
among the Products' executables of the line that count as installed and can
be started, Foxhound before Collie, then the newest version. A refusal's
`fix` names Foxhound only where it would not be.

It differs from the engine's where ADR 0098's note of 2026-11-21 says:
`browser_cache.max_unused_age` is its default, the shared stores' lowest
minor is not read, a copy counts as present by its folder, the cache
`GOODFETCH_BROWSERS_PATH` names is read even where it cannot be written,
and the list is read only in the form the engine writes it. TypeScript's
`installed-executables.ts` is its twin. Blocking: it reads files.
"""

from __future__ import annotations

import json
import os
import re
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from typing import TYPE_CHECKING, NoReturn, TypeVar, cast

from goodfetch_collie_client._framing import object_members
from goodfetch_collie_client.generated.models import ArtifactKind, Product

if TYPE_CHECKING:
    from collections.abc import Mapping

MAX_UNUSED_AGE = timedelta(days=30)
"""`browser_cache.max_unused_age`'s default: a gone executable counts for so long."""

BROWSERS_PATH_VARIABLE = "GOODFETCH_BROWSERS_PATH"

U64_MAX = 2**64 - 1

_ARTIFACT_KINDS = frozenset(kind.value for kind in ArtifactKind)
"""Every kind of the Browser cache's artifacts, from the generated `ArtifactKind`."""

_PRODUCTS = frozenset(product.value for product in Product)
"""Every Product's mark, from the generated `Product`."""

_EPOCH = datetime(1970, 1, 1, tzinfo=UTC)

_FORMAT_VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")

_INSTANT = re.compile(
    r"([0-9]{4})-([0-9]{2})-([0-9]{2})[Tt ]([0-9]{2}):([0-9]{2}):([0-9]{2})"
    r"(?:\.([0-9]{1,9}))?(?:[Zz]|([+-])([0-9]{2}):([0-9]{2}))"
)

_PRODUCT_VERSION = re.compile(
    r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
)

_NUMBER = re.compile(r"[0-9]+")


@dataclass(frozen=True)
class PickedClient:
    """The Client whose executable is asked about."""

    product: Product
    """Its Product."""
    version: str
    """Its Product version."""


def would_be_the_pick(
    client: PickedClient, home: str, env: Mapping[str, str], now: datetime
) -> bool:
    """Whether `client`'s executable would be its major's pick at `now`.

    Recorded in `home`'s list as it would be on running: no executable of
    its line that counts as installed and can be started outranks it. A
    list that cannot be read names none, as the engine reads it.
    """
    own = _product_version(client.version)
    if own is None:
        return True
    cache = _browser_cache(home, env)
    return not any(
        entry.kind == "product_executable"
        and _same_line(entry.version, own)
        and _outranks(entry, client.product, own)
        and _startable(entry, cache, now)
        for entry in _installed(home)
    )


@dataclass(frozen=True)
class _Instant:
    """A point in time in RFC 3339: whole seconds since the epoch, and a fraction."""

    epoch_seconds: int
    fraction: str
    """Its fraction of a second's digits, without trailing zeros."""

    def nanoseconds(self) -> int:
        """Nanoseconds since the epoch."""
        return self.epoch_seconds * 10**9 + int(self.fraction.ljust(9, "0"))


@dataclass(frozen=True)
class _ProductVersion:
    """A Product version `major.minor.patch` with an optional pre-release (ADR 0090)."""

    text: str
    major: int
    minor: int
    patch: str
    """Its patch number's canonical digits, unbounded, as the engine keeps them."""
    pre_release: tuple[str, ...] | None


@dataclass(frozen=True)
class _ArtifactId:
    kind: ArtifactKind
    name: str
    build: str


@dataclass(frozen=True)
class _Entry:
    """One entry of the list, as the engine reads it."""

    kind: str
    """`product_executable` or `linked_program`."""
    product: Product
    version: _ProductVersion
    path: str
    size: int
    modified_at: _Instant
    copy: _ArtifactId | None
    last_seen: _Instant


def _refuse_constant(name: str) -> NoReturn:
    # JSON has no NaN or Infinity, which Python's parser takes by default.
    message = f"{name} is no JSON value"
    raise ValueError(message)


def _items(value: object) -> list[object] | None:
    """`value`'s items when it is a JSON array, else `None`."""
    return cast("list[object]", value) if isinstance(value, list) else None


def _installed(home: str) -> list[_Entry]:
    """The entries of `home`'s list it can read.

    None when the list is missing, is not a JSON object in valid UTF-8 or is
    of another major than 0 (its rule is FrozenMajor), and only the entries
    in the shape the engine reads.
    """
    try:
        with open(os.path.join(home, "installed-executables.json"), "rb") as file:
            text = file.read().decode("utf-8")
        document = object_members(json.loads(text, parse_constant=_refuse_constant))
    except (OSError, ValueError, RecursionError):
        # Nested deeper than Python's parser goes, the engine's goes no deeper.
        return []
    if document is None or _format_major(document.get("format_version")) != 0:
        return []
    executables = _items(document.get("executables")) or []
    return [entry for value in executables if (entry := _entry_of(value)) is not None]


def _format_major(value: object) -> int | None:
    """The major of a format version `major.minor`, each canonical, up to 65535."""
    if not isinstance(value, str):
        return None
    match = _FORMAT_VERSION.fullmatch(value)
    if match is None:
        return None
    major, minor = _bounded(match[1], 0xFFFF), _bounded(match[2], 0xFFFF)
    return major if minor is not None else None


def _entry_of(value: object) -> _Entry | None:
    members = object_members(value)
    if members is None:
        return None
    kind = members.get("kind")
    product = members.get("product")
    path = members.get("path")
    size = members.get("size")
    pins = _items(members.get("pins"))
    version = _product_version(members.get("product_version"))
    modified_at = _instant(members.get("modified_at"))
    last_seen = _instant(members.get("last_seen"))
    copy = members.get("copy")
    copy_id = None if copy is None else _artifact_id(copy)
    if (
        not isinstance(kind, str)
        or kind not in ("product_executable", "linked_program")
        or not isinstance(product, str)
        or product not in _PRODUCTS
        or version is None
        or not isinstance(path, str)
        or not isinstance(size, int)
        or isinstance(size, bool)
        or not 0 <= size <= U64_MAX
        or modified_at is None
        or last_seen is None
        or pins is None
        or not all(_artifact_id(pin) is not None for pin in pins)
        or (copy is not None and copy_id is None)
    ):
        return None
    return _Entry(
        kind=kind,
        product=Product(product),
        version=version,
        path=path,
        size=size,
        modified_at=modified_at,
        copy=copy_id,
        last_seen=last_seen,
    )


def _artifact_id(value: object) -> _ArtifactId | None:
    members = object_members(value)
    if members is None:
        return None
    kind, name, build = members.get("kind"), members.get("name"), members.get("build")
    if (
        not isinstance(kind, str)
        or kind not in _ARTIFACT_KINDS
        or not isinstance(name, str)
        or not isinstance(build, str)
    ):
        return None
    return _ArtifactId(ArtifactKind(kind), name, build)


def _instant(value: object) -> _Instant | None:
    """`value` read as RFC 3339, as the engine reads it.

    A date and time that exist, a leap second read as the second before it,
    at most nine digits of fraction; `None` when it is not.
    """
    if not isinstance(value, str):
        return None
    match = _INSTANT.fullmatch(value)
    if match is None:
        return None
    year, month, day, hour, minute, second = (
        int(match[group]) for group in range(1, 7)
    )
    offset_hours = int(match[9] or 0)
    offset_minutes = int(match[10] or 0)
    if (
        not 1 <= month <= 12
        or not 1 <= day <= _days_in(year, month)
        or hour > 23
        or minute > 59
        or second > 60
        or offset_hours > 23
        or offset_minutes > 59
    ):
        return None
    offset = (offset_hours * 60 + offset_minutes) * 60 * (-1 if match[8] == "-" else 1)
    seconds = (
        _days_from_civil(year, month, day) * 86400
        + hour * 3600
        + minute * 60
        + min(second, 59)
    )
    return _Instant(seconds - offset, (match[7] or "").rstrip("0"))


def _days_in(year: int, month: int) -> int:
    """The days of `month` (1 to 12) in `year` of the proleptic Gregorian calendar."""
    leap = year % 4 == 0 and (year % 100 != 0 or year % 400 == 0)
    return (31, 29 if leap else 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31)[month - 1]


def _days_from_civil(year: int, month: int, day: int) -> int:
    """The days from 1970-01-01 to that date of the proleptic Gregorian calendar."""
    year -= month <= 2
    era = year // 400
    year_of_era = year - era * 400
    day_of_year = (153 * (month + (-3 if month > 2 else 9)) + 2) // 5 + day - 1
    day_of_era = year_of_era * 365 + year_of_era // 4 - year_of_era // 100 + day_of_year
    return era * 146097 + day_of_era - 719468


def _outranks(entry: _Entry, product: Product, version: _ProductVersion) -> bool:
    """Whether `entry` ranks above `product` at `version`: Foxhound, then newest."""
    by_product = _product_rank(entry.product) - _product_rank(product)
    return (
        _compare_versions(entry.version, version) > 0
        if by_product == 0
        else by_product > 0
    )


def _product_rank(product: Product) -> int:
    """Foxhound, which contains Collie, above Collie."""
    return 1 if product is Product.foxhound else 0


def _startable(entry: _Entry, cache: str, now: datetime) -> bool:
    """Whether `entry` counts as installed at `now` and can be started.

    Its file is still at its path at its size and modification time, or it
    was seen within `browser_cache.max_unused_age` and its versioned copy is
    in the Browser cache at `cache`.
    """
    if _still_at_path(entry):
        return True
    now_nanoseconds = (now - _EPOCH) // timedelta(microseconds=1) * 1000
    elapsed = now_nanoseconds - entry.last_seen.nanoseconds()
    return (
        elapsed < MAX_UNUSED_AGE // timedelta(microseconds=1) * 1000
        and entry.copy is not None
        and _copy_present(cache, entry.copy)
    )


def _still_at_path(entry: _Entry) -> bool:
    """Whether what is at `entry`'s path has its size and its modification time.

    The time cut to whole seconds toward zero, as the engine cuts it, so a
    file system with coarser times reads the same.
    """
    try:
        found = os.stat(entry.path)
    except (OSError, ValueError):
        return False
    nanoseconds = found.st_mtime_ns
    whole = nanoseconds // 10**9 if nanoseconds >= 0 else -(-nanoseconds // 10**9)
    return (
        found.st_size == entry.size
        and entry.modified_at.fraction == ""
        and whole == entry.modified_at.epoch_seconds
    )


def _copy_present(cache: str, copy: _ArtifactId) -> bool:
    """Whether the copy has its entry and its files in the Browser cache at `cache`."""
    folder = os.path.join(cache, copy.kind, copy.name, copy.build)
    return os.path.isfile(os.path.join(folder, "entry.json")) and os.path.isdir(
        os.path.join(folder, "files")
    )


def _browser_cache(home: str, env: Mapping[str, str]) -> str:
    """The cache `GOODFETCH_BROWSERS_PATH` names, else `browsers` in Goodfetch home."""
    return env.get(BROWSERS_PATH_VARIABLE) or os.path.join(home, "browsers")


def _product_version(value: object) -> _ProductVersion | None:
    """A Product version as the engine reads it: its major and minor fit 64 bits."""
    if not isinstance(value, str):
        return None
    match = _PRODUCT_VERSION.fullmatch(value)
    if match is None:
        return None
    major, minor = _bounded(match[1], U64_MAX), _bounded(match[2], U64_MAX)
    if major is None or minor is None:
        return None
    pre_release = None if match[4] is None else tuple(match[4].split("."))
    return _ProductVersion(value, major, minor, match[3], pre_release)


def _same_line(left: _ProductVersion, right: _ProductVersion) -> bool:
    """Whether both are of one line: a major, or a `0.y` minor (ADR 0030)."""
    return left.major == right.major and (left.major != 0 or left.minor == right.minor)


def _compare_versions(left: _ProductVersion, right: _ProductVersion) -> int:
    """Semantic Versioning's precedence, as the engine orders Product versions.

    The numbers, then a release above its pre-releases, compared identifier
    by identifier; then the text, so only one version equals another.
    """
    return (
        _compare(left.major, right.major)
        or _compare(left.minor, right.minor)
        or _compare(len(left.patch), len(right.patch))
        or _compare(left.patch, right.patch)
        or _compare_pre_releases(left.pre_release, right.pre_release)
        or _compare(left.text, right.text)
    )


def _compare_pre_releases(
    left: tuple[str, ...] | None, right: tuple[str, ...] | None
) -> int:
    if left is None or right is None:
        return (left is None) - (right is None)
    for left_identifier, right_identifier in zip(left, right, strict=False):
        order = _compare_identifiers(left_identifier, right_identifier)
        if order != 0:
            return order
    return _compare(len(left), len(right))


def _compare_identifiers(left: str, right: str) -> int:
    """Numbers numerically and below words, words by their bytes."""
    left_number, right_number = _number_of(left), _number_of(right)
    if left_number is not None and right_number is not None:
        return _compare(left_number, right_number)
    if left_number is not None or right_number is not None:
        return 1 if left_number is None else -1
    return _compare(left, right)


def _number_of(identifier: str) -> int | None:
    """An identifier of digits alone that fits 64 bits, as the engine reads a number."""
    if _NUMBER.fullmatch(identifier) is None:
        return None
    return _bounded(identifier, U64_MAX)


def _bounded(digits: str, limit: int) -> int | None:
    """`digits` as a number when it is at most `limit`, else `None`.

    Leading zeros aside, more digits than `limit` has are past it unread:
    Python's `int` refuses a string of more than 4300 digits.
    """
    significant = digits.lstrip("0") or "0"
    if len(significant) > len(str(limit)):
        return None
    number = int(significant)
    return number if number <= limit else None


_Ordered = TypeVar("_Ordered", int, str)


def _compare(left: _Ordered, right: _Ordered) -> int:
    return (left > right) - (left < right)
