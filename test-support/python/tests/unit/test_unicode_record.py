"""The shared Unicode record (ADR 0073)."""

from collections.abc import Callable
from pathlib import Path

from goodfetch_test_support import UnicodeField, unicode_record


def round_trip(folder: Path, read: Callable[[Path], str]) -> list[UnicodeField]:
    """Writes each field to a file in `folder` and reads it back through `read`."""
    read_back: list[UnicodeField] = []
    for field in unicode_record().fields:
        path = folder / f"{field.name}.txt"
        path.write_bytes(field.text.encode("utf-8"))
        read_back.append(UnicodeField(field.name, read(path)))
    return read_back


def test_the_unicode_record_holds_adr_0073s_hard_text() -> None:
    """The record holds each kind of hard text ADR 0073 lists."""
    record = unicode_record()

    assert record.fields == (
        UnicodeField(
            "bidi",
            "\u05e9\u05dc\u05d5\u05dd \u200fworld\u200e "
            "\u0645\u0631\u062d\u0628\u0627 "
            "\u2067\u05e2\u05d1\u05e8\u05d9\u05ea\u2069 end",
        ),
        UnicodeField("nfc", "caf\u00e9"),
        UnicodeField("nfd", "cafe\u0301"),
        UnicodeField("zwj_emoji", "\U0001f469\u200d\U0001f469\u200d\U0001f467"),
        UnicodeField("astral", "\U0001d11e"),
        UnicodeField("zero_width_joiner", "a\u200db"),
        UnicodeField("bom", "\ufeffleads with a byte order mark"),
        UnicodeField("tab_and_newline", "one\ttwo\nthree"),
    )


def test_the_unicode_record_round_trips_byte_for_byte_through_a_file(
    tmp_path: Path,
) -> None:
    """Each field reads back from a file with the bytes it was written with."""
    read_back = round_trip(tmp_path, lambda path: path.read_bytes().decode("utf-8"))

    assert tuple(read_back) == unicode_record().fields


def test_a_reader_that_drops_a_leading_byte_order_mark_fails_the_round_trip(
    tmp_path: Path,
) -> None:
    """A `utf-8-sig` reader drops the `bom` field's mark, which the record catches."""
    read_back = round_trip(tmp_path, lambda path: path.read_text("utf-8-sig"))

    assert tuple(read_back) != unicode_record().fields
