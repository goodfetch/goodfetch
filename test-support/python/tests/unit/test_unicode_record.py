"""The shared Unicode record (ADR 0073)."""

from goodfetch_test_support import UnicodeField, unicode_record


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
