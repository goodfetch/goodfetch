"""A response's body as text and as JSON (ADR 0119), as Rust's client reads it.

`text()` decodes with the response's own encoding, a navigation's document
with the one its document was decoded with, any other with its BOM, its
`Content-Type` charset, Chrome's `<meta>` prescan for HTML or the XML
declaration for an XML type, then UTF-8; `json()` decodes UTF-8 and
ignores a BOM. Rust's `response/tests.rs` and `goodfetch_encoding`'s tests
are its twins, and TypeScript's `decoding.test.ts`.
"""

from __future__ import annotations

import pytest
from goodfetch_collie_client._decoding import (
    UTF_8,
    WINDOWS_1252,
    Kind,
    decode,
    json_of,
    kind_of,
    media_type,
    prescan,
    text,
    xml_declaration,
)

SHIFT_JIS_BYTES = bytes([0x93, 0xFA, 0x96, 0x7B, 0x8C, 0xEA])
"""日本語 in `Shift_JIS`."""

E_ACUTE = bytes([0xE9])
"""`é` in windows-1252."""

NIHON = bytes([0x93, 0xFA, 0x96, 0x7B])
"""日本 in `Shift_JIS`, which is no UTF-8."""

SHIFT_JIS = "Shift_JIS"
WINDOWS_1251 = "windows-1251"


def with_shift_jis(head: str, tail: str) -> bytes:
    """`head`, then the `Shift_JIS` bytes of 日本語, then `tail`."""
    return head.encode() + SHIFT_JIS_BYTES + tail.encode()


def with_e_acute(head: str, tail: str) -> bytes:
    """`head`, then the windows-1252 byte of `é`, then `tail`."""
    return head.encode() + E_ACUTE + tail.encode()


def past_1024(line: str) -> str:
    """`line` repeated past the first 1,024 bytes of a document."""
    return line * (1024 // len(line) + 2)


def html(head: str, body: str) -> bytes:
    """An HTML document of `head` and `body`, with the `Shift_JIS` 日本 in its body."""
    start = f"<!doctype html><html><head>{head}</head><body>{body}<p>"
    return start.encode() + NIHON + b"</p></body></html>"


@pytest.mark.covers("story:11-85")
def test_a_navigations_document_decodes_with_the_encoding_the_browser_used() -> None:
    """The document's own encoding wins over the `Content-Type` charset."""
    body = with_shift_jis("<p>", "</p>")

    assert text(body, "Shift_JIS", "text/html; charset=utf-8") == "<p>日本語</p>"


@pytest.mark.covers("story:11-85")
def test_a_bom_wins_over_the_content_type_charset_and_is_left_out() -> None:
    """A UTF-8 BOM names the encoding and is no text."""
    body = b"\xef\xbb\xbf" + "café".encode()

    assert text(body, None, "text/plain; charset=windows-1252") == "café"


@pytest.mark.covers("story:11-85")
@pytest.mark.parametrize(
    "content_type",
    [
        pytest.param("text/plain; charset=Shift_JIS", id="plain"),
        pytest.param('text/plain; charset="shift_jis"', id="quoted"),
        pytest.param("text/plain; format=flowed; Charset=sjis", id="among parameters"),
    ],
)
def test_a_legacy_content_type_charset_decodes_the_body(content_type: str) -> None:
    """A `Content-Type` charset in any spelling decodes the body."""
    assert text(SHIFT_JIS_BYTES, None, content_type) == "日本語"


@pytest.mark.covers("story:11-85")
@pytest.mark.parametrize(
    ("head", "tail"),
    [
        pytest.param(
            '<!doctype html><html><head><meta charset="shift_jis"><title>',
            "</title>",
            id="charset attribute",
        ),
        pytest.param(
            '<html><head><meta http-equiv="Content-Type"'
            ' content="text/html; charset=Shift_JIS"><title>',
            "</title>",
            id="pragma",
        ),
        pytest.param(
            f"<html><head>{'<meta name=k content=v>' * 50}<meta charset=sjis><title>",
            "</title>",
            id="late among head tags",
        ),
    ],
)
def test_an_html_body_declaring_its_encoding_in_a_meta_decodes_with_it(
    head: str, tail: str
) -> None:
    """The `<meta>` reads as Chrome's prescan does, past 1,024 bytes among head tags."""
    body = with_shift_jis(head, tail)

    assert text(body, None, "text/html") == f"{head}日本語{tail}"


@pytest.mark.covers("story:11-85")
def test_a_content_type_charset_no_encoding_has_gives_way_to_the_meta() -> None:
    """An unknown `Content-Type` charset reads as none."""
    head = '<meta charset="windows-1252"><p>'
    body = with_e_acute(head, "</p>")

    assert text(body, None, "text/html; charset=no-such-encoding") == f"{head}é</p>"


@pytest.mark.covers("story:11-85")
@pytest.mark.parametrize(
    "content_type",
    [
        pytest.param("application/xml", id="xml"),
        pytest.param("text/xml", id="text xml"),
        pytest.param("application/atom+xml", id="suffixed"),
        pytest.param("text/html", id="html without a meta"),
    ],
)
def test_an_xml_declaration_at_the_start_decodes_the_body(content_type: str) -> None:
    """An XML type, or HTML with no `<meta>`, reads the XML declaration."""
    head = '<?xml version="1.0" encoding="windows-1252"?><a>'
    body = with_e_acute(head, "</a>")

    assert text(body, None, content_type) == f"{head}é</a>"


@pytest.mark.covers("story:11-85")
@pytest.mark.parametrize(
    "content_type",
    [pytest.param("text/plain", id="another type"), pytest.param(None, id="no type")],
)
def test_a_body_of_another_type_reads_no_declaration_and_decodes_as_utf_8(
    content_type: str | None,
) -> None:
    """Any other type decodes as UTF-8, a byte that does not decode U+FFFD."""
    head = '<?xml version="1.0" encoding="windows-1252"?><a>'
    body = with_e_acute(head, "</a>")

    assert text(body, None, content_type) == f"{head}�</a>"


@pytest.mark.covers("story:11-85")
@pytest.mark.parametrize(
    ("body", "encoding", "decoded"),
    [
        pytest.param(b"\xff\xfea\x00", "UTF-16LE", "a", id="a UTF-16 BOM is left out"),
        pytest.param(b"\x80\x81", "windows-1252", "€\x81", id="a C1 control"),
        pytest.param(b"\xae", "KOI8-U", "ў", id="the standard's KOI8-U"),
        pytest.param(b"\xf8", "x-user-defined", "", id="x-user-defined"),
        pytest.param(b"abc", "replacement", "�", id="replacement"),
        pytest.param(
            b"\xa1", "ISO-8859-6", "�", id="a byte the encoding leaves undefined"
        ),
    ],
)
def test_an_encoding_decodes_as_the_encoding_standard_maps_it(
    body: bytes, encoding: str, decoded: str
) -> None:
    """Each encoding decodes as `encoding_rs` does, where Python's codec departs."""
    assert decode(body, encoding) == decoded


@pytest.mark.covers("story:11-85")
def test_json_ignores_a_leading_bom_and_decodes_utf_8_whatever_the_charset() -> None:
    """A BOM before JSON is no JSON error."""
    body = b"\xef\xbb\xbf" + '{"name":"café"}'.encode()

    assert json_of(body) == {"name": "café"}


@pytest.mark.covers("story:11-85")
@pytest.mark.parametrize(
    "body",
    [pytest.param(b"<html></html>", id="html"), pytest.param(b"[NaN]", id="a NaN")],
)
def test_json_of_a_body_that_is_no_json_fails(body: bytes) -> None:
    """HTML, or a `NaN` Python's own reader would take, is no JSON."""
    with pytest.raises(ValueError, match=r"."):
        json_of(body)


@pytest.mark.covers("story:6-233")
@pytest.mark.parametrize(
    ("value", "essence", "charset"),
    [
        pytest.param(
            "text/html; charset=Shift_JIS", "text/html", "Shift_JIS", id="a charset"
        ),
        pytest.param(
            'Text/HTML;Charset="shift_jis"',
            "text/html",
            "shift_jis",
            id="quoted in another case",
        ),
        pytest.param(
            "application/xhtml+xml; q=1 ; charset = utf-8",
            "application/xhtml+xml",
            "utf-8",
            id="another parameter first",
        ),
        pytest.param(" text/plain ", "text/plain", None, id="none"),
    ],
)
def test_a_content_type_names_its_essence_and_charset(
    value: str, essence: str, charset: str | None
) -> None:
    """The essence, lowercase, and the `charset` parameter, in any case and unquoted."""
    assert media_type(value) == (essence, charset)


@pytest.mark.covers("story:5-129")
@pytest.mark.parametrize(
    ("essence", "kind"),
    [
        pytest.param("text/html", "html", id="html"),
        pytest.param("application/xhtml+xml", "xml", id="xhtml"),
        pytest.param("text/xml", "xml", id="text xml"),
        pytest.param("image/svg+xml", "xml", id="a plus xml type"),
        pytest.param("application/json", "other", id="json"),
    ],
)
def test_a_type_reads_its_own_declaration(essence: str, kind: Kind) -> None:
    """HTML reads the `<meta>` prescan, an XML type its declaration, any other none."""
    assert kind_of(essence) == kind


@pytest.mark.covers("story:5-129")
@pytest.mark.parametrize(
    ("body", "expected"),
    [
        pytest.param(html("<meta charset=shift_jis>", ""), SHIFT_JIS, id="a charset"),
        pytest.param(
            html("<META CharSet='Shift_JIS'>", ""),
            SHIFT_JIS,
            id="a quoted charset in any case",
        ),
        pytest.param(
            html(
                "<meta content=\"text/html;charset='shift_jis'\""
                " http-equiv=content-type>",
                "",
            ),
            SHIFT_JIS,
            id="a pragma",
        ),
        pytest.param(
            html('<meta content="text/html; charset=shift_jis">', ""),
            None,
            id="a content without its pragma",
        ),
        pytest.param(html("<meta charset=utf-16le>", ""), UTF_8, id="a UTF-16 label"),
        pytest.param(
            html("<meta charset=x-user-defined>", ""), WINDOWS_1252, id="x-user-defined"
        ),
        pytest.param(
            html("<meta charset=klingon><meta charset=windows-1251>", ""),
            WINDOWS_1251,
            id="an unknown label then a known one",
        ),
        pytest.param(
            html("<!-- <meta charset=shift_jis> -->", ""), None, id="in a comment"
        ),
        pytest.param(
            html("<title><meta charset=shift_jis></title>", ""), None, id="in a title"
        ),
        pytest.param(
            html("<script>'<meta charset=shift_jis>'</script>", ""),
            None,
            id="in a script",
        ),
        pytest.param(
            html(
                f"{past_1024('<meta name=k content=v>')}<meta charset=windows-1251>", ""
            ),
            WINDOWS_1251,
            id="late among head tags",
        ),
        pytest.param(
            html("", "<meta charset=windows-1251>"),
            WINDOWS_1251,
            id="after the body in the first 1024 bytes",
        ),
        pytest.param(
            html("", f"{past_1024('<p>filler</p>')}<meta charset=windows-1251>"),
            None,
            id="after the body past 1024 bytes",
        ),
        pytest.param(b"<meta charset=shift_jis", None, id="an unclosed tag"),
    ],
)
def test_the_prescan_reads_a_meta_as_chrome_does(
    body: bytes, expected: str | None
) -> None:
    """Blink's `HTMLMetaCharsetParser`: 1,024 bytes, then on while head tags appear.

    UTF-16 reads as UTF-8 and `x-user-defined` as windows-1252; a
    declaration in a comment, a title or a script is none, and an unknown
    label is skipped.
    """
    assert prescan(body) == expected


@pytest.mark.covers("story:5-129")
@pytest.mark.parametrize(
    ("body", "expected"),
    [
        pytest.param(
            b'<?xml version="1.0" encoding="Shift_JIS"?><a/>',
            SHIFT_JIS,
            id="double quoted",
        ),
        pytest.param(
            b"<?xml version='1.0' encoding = 'windows-1251' ?>",
            WINDOWS_1251,
            id="single quoted with spaces",
        ),
        pytest.param(b'<?xml version="1.0" encoding="UTF-16"?>', UTF_8, id="UTF-16"),
        pytest.param(b'<?xml version="1.0"?><a/>', None, id="no encoding"),
        pytest.param(b'<?xml version="1.0" encoding=Shift_JIS?>', None, id="unquoted"),
        pytest.param(
            b' <?xml version="1.0" encoding="Shift_JIS"?>', None, id="not at the start"
        ),
        pytest.param(b'<?xml version="1.0" encoding="Shift_JIS"', None, id="unclosed"),
    ],
)
def test_the_xml_declaration_names_an_encoding(
    body: bytes, expected: str | None
) -> None:
    """Blink's `TextResourceDecoder`: at the start, quoted, UTF-16 as UTF-8."""
    assert xml_declaration(body) == expected
