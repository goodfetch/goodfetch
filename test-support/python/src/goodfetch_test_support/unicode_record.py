"""The shared Unicode record (ADR 0073).

It is read from `test-support/fixtures/unicode-record.json`, the one file
Rust and TypeScript read too, so every Sink conformance case and Export format
checks the same hard text round-trips byte for byte.
"""

from dataclasses import dataclass
from typing import cast

from goodfetch_test_support.schemas import shared_fixture


@dataclass(frozen=True)
class UnicodeField:
    """One field of the Unicode record."""

    name: str
    """The field's name, in snake case."""
    text: str
    """Its text."""


@dataclass(frozen=True)
class UnicodeRecord:
    """The Unicode record: one Record's fields, each a kind of hard text.

    The fields come in the committed file's order: `bidi`, `nfc`, `nfd`,
    `zwj_emoji`, `astral`, `zero_width_joiner`, `bom` and `tab_and_newline`.
    """

    fields: tuple[UnicodeField, ...]


def unicode_record() -> UnicodeRecord:
    """The committed Unicode record.

    Returns:
        The record, read from `test-support/fixtures/unicode-record.json`.
    """
    record = cast(
        "dict[str, list[dict[str, str]]]",
        shared_fixture("unicode-record.json", "unicode-record.schema.json"),
    )
    return UnicodeRecord(
        tuple(UnicodeField(field["name"], field["text"]) for field in record["fields"])
    )
