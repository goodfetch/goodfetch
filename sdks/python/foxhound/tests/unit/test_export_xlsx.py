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

import openpyxl
import pytest
from goodfetch_test_support import WORKSPACE, unicode_record

SNAPSHOTS = WORKSPACE / "crates/foxhound/goodfetch-foxhound/tests/it/snapshots"
"""The insta snapshots of `goodfetch-foxhound`'s integration tests."""

WORKBOOK = SNAPSHOTS / "it__exports__d07_books.snap.xlsx"
"""The D07 Export's workbook, as the Rust acceptance test committed it."""


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


@pytest.mark.covers("format:export")
def test_openpyxl_reads_each_d07_record_cell_as_its_own_type() -> None:
    """A string, a number, a boolean, an empty cell and JSON text read back."""
    text = {field.name: field.text for field in unicode_record().fields}
    # fmt: off
    expected: list[list[object]] = [
        ["page", "title", "astral", "bidi", "bom", "nfc", "nfd",
         "tab_and_newline", "zero_width_joiner", "zwj_emoji", "author",
         "in_stock", "note", "price", "tags"],
        [1, "Café 1", None, None, None, None, None,
         None, None, None, None,
         None, None, None, None],
        [None, None, text["astral"], text["bidi"], text["bom"], text["nfc"],
         text["nfd"], text["tab_and_newline"], text["zero_width_joiner"],
         text["zwj_emoji"], None, None, None, None, None],
        [3, "Café 3", None, None, None, None, None,
         None, None, None, '{"name":"Zoé"}',
         True, None, 12.5, '["roman","poche"]'],
    ]
    # fmt: on

    names, rows = read(WORKBOOK.read_bytes())

    assert (names, typed(rows)) == (["books"], typed(expected))
