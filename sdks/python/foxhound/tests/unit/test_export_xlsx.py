"""Pitfall D07's XLSX Export, opened by a spreadsheet library (issue 1679).

The Rust D07 acceptance test of `goodfetch-foxhound`'s `tests/it/exports.rs`
crawls the `tables` Dataset on Foxhound's executable, exports it as XLSX and
keeps the workbook as an insta binary snapshot, which fails when the engine
writes other bytes. Here openpyxl opens that committed workbook and reads back
the sheet's name, the header row and each Record's cells, each as its own
type. A Rust spreadsheet library cannot do it: calamine turns on another
deflate backend for the whole shared build, which changes the Proving ground's
compressed bodies.
"""

from __future__ import annotations

import io
import json
import zipfile

import openpyxl
import pytest
from goodfetch_test_support import WORKSPACE, unicode_record

SNAPSHOTS = WORKSPACE / "crates/foxhound/goodfetch-foxhound/tests/it/snapshots"
"""The insta snapshots of `goodfetch-foxhound`'s integration tests."""

WORKBOOK = SNAPSHOTS / "it__exports__d07_books.snap.xlsx"
"""The D07 Export's workbook, as the Rust acceptance test committed it."""

COLUMNS = (
    "page",
    "title",
    "astral",
    "bidi",
    "bom",
    "nfc",
    "nfd",
    "tab_and_newline",
    "zero_width_joiner",
    "zwj_emoji",
    "author",
    "in_stock",
    "note",
    "price",
    "tags",
)
"""Every field the Records carry, in first-seen order."""


def read(workbook: bytes) -> tuple[list[str], list[list[object]]]:
    """Opens `workbook` with openpyxl.

    Args:
        workbook: An XLSX file's bytes.

    Returns:
        The sheets' names and the first sheet's rows, its header row first.
    """
    book = openpyxl.load_workbook(io.BytesIO(workbook))
    rows: list[list[object]] = [
        list(row) for row in book.worksheets[0].iter_rows(values_only=True)
    ]
    return book.sheetnames, rows


def typed(rows: list[list[object]]) -> list[list[tuple[type, object]]]:
    """Pairs each cell with its type, since `True == 1` in Python."""
    return [[(type(cell), cell) for cell in row] for row in rows]


def expected_rows() -> list[list[object]]:
    """The rows D07's three Records give, each cell of its own type."""
    unicode: dict[str, object] = {
        field.name: field.text for field in unicode_record().fields
    }
    records: list[dict[str, object]] = [
        {"page": 1, "title": "Café 1"},
        unicode,
        {
            "author": '{"name":"Zoé"}',
            "in_stock": True,
            "page": 3,
            "price": 12.5,
            "tags": '["roman","poche"]',
            "title": "Café 3",
        },
    ]
    header: list[object] = list(COLUMNS)
    return [header, *([record.get(name) for name in COLUMNS] for record in records)]


@pytest.mark.covers("format:export")
def test_openpyxl_reads_each_d07_record_cell_as_its_own_type() -> None:
    """A string, a number, a boolean, an empty cell and JSON text read back."""
    names, rows = read(WORKBOOK.read_bytes())

    assert (names, typed(rows)) == (["books"], typed(expected_rows()))
    author = rows[3][COLUMNS.index("author")]
    assert isinstance(author, str)
    assert json.loads(author) == {"name": "Zoé"}


def test_a_workbook_openpyxl_cannot_open_fails_the_read() -> None:
    """The read raises, so the test above fails, on a workbook cut short."""
    whole = WORKBOOK.read_bytes()

    with pytest.raises(zipfile.BadZipFile):
        read(whole[: len(whole) // 2])
