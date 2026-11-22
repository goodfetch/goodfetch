"""A response body's text and JSON (ADR 0119), as Rust's client reads them.

A navigation's document decodes with the encoding its document was decoded
with, the browser's `document.characterSet`; any other response with its
BOM, its `Content-Type` charset, the encoding it declares in its own bytes
(for HTML Chrome's `<meta>` prescan then an XML declaration at the very
start, for an XML type that declaration), and UTF-8 last. A byte that does
not decode becomes U+FFFD. JSON is UTF-8 whatever the charset, a leading
BOM ignored. Rust's `goodfetch_encoding` crate is the reading this module
follows; TypeScript's `decoding.ts` is its twin.

The labels and the single-byte tables are the Encoding Standard's, as
`encoding_rs` 0.8.44 holds them: Python's own codecs decode each encoding,
with the few bytes where a codec departs from the standard's table mapped
as the standard maps them. The prescan reads the bytes with the HTML
standard's tokenizer states written here, where Rust's runs html5ever's: a
character reference is read as its own characters, and a script's escaped
text as plain script text, which no `<meta>` read depends on.
"""

from __future__ import annotations

import codecs
import functools
import json
from typing import Final, Literal

from goodfetch_collie_client.json_values import JSONValue

UNCONDITIONAL: Final = 1024
"""How many bytes the prescan always reads, as Blink's `kBytesToCheckUnconditionally`."""

UTF_8: Final = "UTF-8"
UTF_16LE: Final = "UTF-16LE"
UTF_16BE: Final = "UTF-16BE"
WINDOWS_1252: Final = "windows-1252"
X_USER_DEFINED: Final = "x-user-defined"
REPLACEMENT: Final = "replacement"

_ENCODINGS: Final[dict[str, tuple[str, ...]]] = {
    "Big5": ("big5", "big5-hkscs", "cn-big5", "csbig5", "x-x-big5"),
    "EUC-JP": ("cseucpkdfmtjapanese", "euc-jp", "x-euc-jp"),
    "EUC-KR": (
        "cseuckr",
        "csksc56011987",
        "euc-kr",
        "iso-ir-149",
        "korean",
        "ks_c_5601-1987",
        "ks_c_5601-1989",
        "ksc5601",
        "ksc_5601",
        "windows-949",
    ),
    "gb18030": ("gb18030",),
    "GBK": (
        "chinese",
        "csgb2312",
        "csiso58gb231280",
        "gb2312",
        "gb_2312",
        "gb_2312-80",
        "gbk",
        "iso-ir-58",
        "x-gbk",
    ),
    "IBM866": ("866", "cp866", "csibm866", "ibm866"),
    "ISO-2022-JP": ("csiso2022jp", "iso-2022-jp"),
    "ISO-8859-10": (
        "csisolatin6",
        "iso-8859-10",
        "iso-ir-157",
        "iso8859-10",
        "iso885910",
        "l6",
        "latin6",
    ),
    "ISO-8859-13": ("iso-8859-13", "iso8859-13", "iso885913"),
    "ISO-8859-14": ("iso-8859-14", "iso8859-14", "iso885914"),
    "ISO-8859-15": (
        "csisolatin9",
        "iso-8859-15",
        "iso8859-15",
        "iso885915",
        "iso_8859-15",
        "l9",
    ),
    "ISO-8859-16": ("iso-8859-16",),
    "ISO-8859-2": (
        "csisolatin2",
        "iso-8859-2",
        "iso-ir-101",
        "iso8859-2",
        "iso88592",
        "iso_8859-2",
        "iso_8859-2:1987",
        "l2",
        "latin2",
    ),
    "ISO-8859-3": (
        "csisolatin3",
        "iso-8859-3",
        "iso-ir-109",
        "iso8859-3",
        "iso88593",
        "iso_8859-3",
        "iso_8859-3:1988",
        "l3",
        "latin3",
    ),
    "ISO-8859-4": (
        "csisolatin4",
        "iso-8859-4",
        "iso-ir-110",
        "iso8859-4",
        "iso88594",
        "iso_8859-4",
        "iso_8859-4:1988",
        "l4",
        "latin4",
    ),
    "ISO-8859-5": (
        "csisolatincyrillic",
        "cyrillic",
        "iso-8859-5",
        "iso-ir-144",
        "iso8859-5",
        "iso88595",
        "iso_8859-5",
        "iso_8859-5:1988",
    ),
    "ISO-8859-6": (
        "arabic",
        "asmo-708",
        "csiso88596e",
        "csiso88596i",
        "csisolatinarabic",
        "ecma-114",
        "iso-8859-6",
        "iso-8859-6-e",
        "iso-8859-6-i",
        "iso-ir-127",
        "iso8859-6",
        "iso88596",
        "iso_8859-6",
        "iso_8859-6:1987",
    ),
    "ISO-8859-7": (
        "csisolatingreek",
        "ecma-118",
        "elot_928",
        "greek",
        "greek8",
        "iso-8859-7",
        "iso-ir-126",
        "iso8859-7",
        "iso88597",
        "iso_8859-7",
        "iso_8859-7:1987",
        "sun_eu_greek",
    ),
    "ISO-8859-8": (
        "csiso88598e",
        "csisolatinhebrew",
        "hebrew",
        "iso-8859-8",
        "iso-8859-8-e",
        "iso-ir-138",
        "iso8859-8",
        "iso88598",
        "iso_8859-8",
        "iso_8859-8:1988",
        "visual",
    ),
    "ISO-8859-8-I": ("csiso88598i", "iso-8859-8-i", "logical"),
    "KOI8-R": ("cskoi8r", "koi", "koi8", "koi8-r", "koi8_r"),
    "KOI8-U": ("koi8-ru", "koi8-u"),
    "macintosh": ("csmacintosh", "mac", "macintosh", "x-mac-roman"),
    REPLACEMENT: (
        "csiso2022kr",
        "hz-gb-2312",
        "iso-2022-cn",
        "iso-2022-cn-ext",
        "iso-2022-kr",
        "replacement",
    ),
    "Shift_JIS": (
        "csshiftjis",
        "ms932",
        "ms_kanji",
        "shift-jis",
        "shift_jis",
        "sjis",
        "windows-31j",
        "x-sjis",
    ),
    UTF_16BE: ("unicodefffe", "utf-16be"),
    UTF_16LE: (
        "csunicode",
        "iso-10646-ucs-2",
        "ucs-2",
        "unicode",
        "unicodefeff",
        "utf-16",
        "utf-16le",
    ),
    UTF_8: (
        "unicode-1-1-utf-8",
        "unicode11utf8",
        "unicode20utf8",
        "utf-8",
        "utf8",
        "x-unicode20utf8",
    ),
    "windows-1250": ("cp1250", "windows-1250", "x-cp1250"),
    "windows-1251": ("cp1251", "windows-1251", "x-cp1251"),
    WINDOWS_1252: (
        "ansi_x3.4-1968",
        "ascii",
        "cp1252",
        "cp819",
        "csisolatin1",
        "ibm819",
        "iso-8859-1",
        "iso-ir-100",
        "iso8859-1",
        "iso88591",
        "iso_8859-1",
        "iso_8859-1:1987",
        "l1",
        "latin1",
        "us-ascii",
        "windows-1252",
        "x-cp1252",
    ),
    "windows-1253": ("cp1253", "windows-1253", "x-cp1253"),
    "windows-1254": (
        "cp1254",
        "csisolatin5",
        "iso-8859-9",
        "iso-ir-148",
        "iso8859-9",
        "iso88599",
        "iso_8859-9",
        "iso_8859-9:1989",
        "l5",
        "latin5",
        "windows-1254",
        "x-cp1254",
    ),
    "windows-1255": ("cp1255", "windows-1255", "x-cp1255"),
    "windows-1256": ("cp1256", "windows-1256", "x-cp1256"),
    "windows-1257": ("cp1257", "windows-1257", "x-cp1257"),
    "windows-1258": ("cp1258", "windows-1258", "x-cp1258"),
    "windows-874": (
        "dos-874",
        "iso-8859-11",
        "iso8859-11",
        "iso885911",
        "tis-620",
        "windows-874",
    ),
    "x-mac-cyrillic": ("x-mac-cyrillic", "x-mac-ukrainian"),
    X_USER_DEFINED: ("x-user-defined",),
}
"""Each encoding, by its Encoding Standard name, and its labels."""

_LABELS: Final = {
    label: name for name, labels in _ENCODINGS.items() for label in labels
}

_SINGLE_BYTE: Final = {
    "IBM866": "cp866",
    "ISO-8859-2": "iso8859_2",
    "ISO-8859-3": "iso8859_3",
    "ISO-8859-4": "iso8859_4",
    "ISO-8859-5": "iso8859_5",
    "ISO-8859-6": "iso8859_6",
    "ISO-8859-7": "iso8859_7",
    "ISO-8859-8": "iso8859_8",
    "ISO-8859-8-I": "iso8859_8",
    "ISO-8859-10": "iso8859_10",
    "ISO-8859-13": "iso8859_13",
    "ISO-8859-14": "iso8859_14",
    "ISO-8859-15": "iso8859_15",
    "ISO-8859-16": "iso8859_16",
    "KOI8-R": "koi8_r",
    "KOI8-U": "koi8_u",
    "macintosh": "mac_roman",
    "windows-874": "cp874",
    "windows-1250": "cp1250",
    "windows-1251": "cp1251",
    WINDOWS_1252: "cp1252",
    "windows-1253": "cp1253",
    "windows-1254": "cp1254",
    "windows-1255": "cp1255",
    "windows-1256": "cp1256",
    "windows-1257": "cp1257",
    "windows-1258": "cp1258",
    "x-mac-cyrillic": "mac_cyrillic",
}
"""The Python codec of each single-byte encoding."""

_DEPARTURES: Final[dict[str, dict[int, str]]] = {
    "KOI8-U": {0xAE: "ў", 0xBE: "Ў"},
    "windows-1255": {0xCA: "ֺ"},
}
"""The bytes a codec of `_SINGLE_BYTE` maps otherwise than the standard's table.

Besides these, the standard maps each byte from 0x80 to 0x9F that a
`windows-` codec leaves undefined to the C1 control of that value.
"""

_MULTI_BYTE: Final = {
    "Big5": "big5hkscs",
    "EUC-JP": "euc_jp",
    "EUC-KR": "cp949",
    "gb18030": "gb18030",
    "GBK": "gb18030",
    "ISO-2022-JP": "iso2022_jp",
    "Shift_JIS": "cp932",
}
"""The Python codec of each multi-byte encoding, the standard's decoder's closest."""

_BOMS: Final = (
    (b"\xef\xbb\xbf", UTF_8),
    (b"\xff\xfe", UTF_16LE),
    (b"\xfe\xff", UTF_16BE),
)

_UNICODE: Final = {UTF_8: "utf-8", UTF_16LE: "utf-16-le", UTF_16BE: "utf-16-be"}

_ASCII_LOWER: Final = str.maketrans(
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ", "abcdefghijklmnopqrstuvwxyz"
)

Kind = Literal["html", "xml", "other"]
"""Which declaration in its own bytes a document's type reads."""


def text(body: bytes, document: str | None, content_type: str | None) -> str:
    """`body` as text.

    Decoded with `document`, the label of the encoding a navigation's
    document was decoded with, when it names one; otherwise with the
    encoding `body` and its `Content-Type` value `content_type` declare. A
    BOM is left out, and a byte that does not decode becomes U+FFFD.
    """
    named = None if document is None else for_label(document)
    return decode(body, named if named is not None else of_body(body, content_type))


def json_of(body: bytes) -> JSONValue:
    """`body` as JSON, decoded as UTF-8 whatever its charset, a leading BOM ignored.

    Raises:
        ValueError: The body is not JSON; `NaN` and `Infinity` are none.
    """
    parsed: JSONValue = json.loads(decode(body, UTF_8), parse_constant=_no_constant)
    return parsed


def _no_constant(constant: str) -> JSONValue:
    raise ValueError(f"`{constant}` is not JSON")


def for_label(label: str) -> str | None:
    """The encoding `label` names, as the Encoding Standard's `get an encoding` reads it."""
    if not label.isascii():
        return None
    return _LABELS.get(label.strip("\t\n\f\r ").translate(_ASCII_LOWER))


def of_body(body: bytes, content_type: str | None) -> str:
    """The encoding `body`, whose `Content-Type` value is `content_type`, declares; UTF-8 when none."""
    for bom, encoding in _BOMS:
        if body.startswith(bom):
            return encoding
    essence, charset = ("", None) if content_type is None else media_type(content_type)
    named = None if charset is None else for_label(charset)
    if named is not None:
        return named
    declared_encoding = declared(body, kind_of(essence))
    return UTF_8 if declared_encoding is None else declared_encoding


def decode(body: bytes, encoding: str) -> str:
    """`body` decoded with `encoding`, its own BOM left out, a byte that does not decode U+FFFD."""
    unicode = _UNICODE.get(encoding)
    if unicode is not None:
        bom = next(bom for bom, named in _BOMS if named == encoding)
        return body.removeprefix(bom).decode(unicode, "replace")
    if encoding == REPLACEMENT:
        return "�" if body else ""
    multi_byte = _MULTI_BYTE.get(encoding)
    if multi_byte is not None:
        return body.decode(multi_byte, "replace")
    decoded, _ = codecs.charmap_decode(body, "replace", _table(encoding))
    return decoded


@functools.cache
def _table(encoding: str) -> str:
    """The decoding table of the single-byte `encoding`, U+FFFE for a byte it leaves undefined."""
    if encoding == X_USER_DEFINED:
        return "".join(
            chr(byte if byte < 0x80 else 0xF700 + byte) for byte in range(256)
        )
    codec = _SINGLE_BYTE[encoding]
    departures = _DEPARTURES.get(encoding, {})

    def character(byte: int) -> str:
        departure = departures.get(byte)
        if departure is not None:
            return departure
        try:
            return bytes([byte]).decode(codec)
        except UnicodeDecodeError:
            control = encoding.startswith("windows-") and 0x80 <= byte <= 0x9F
            return chr(byte) if control else "￾"

    return "".join(character(byte) for byte in range(256))


def media_type(value: str) -> tuple[str, str | None]:
    """The essence of the `Content-Type` value `value`, lowercase, and its `charset`, unquoted."""
    essence, *parameters = value.split(";")
    for parameter in parameters:
        name, equals, charset = parameter.partition("=")
        if equals and name.strip().translate(_ASCII_LOWER) == "charset":
            return essence.strip().translate(_ASCII_LOWER), charset.strip().strip('"')
    return essence.strip().translate(_ASCII_LOWER), None


def kind_of(essence: str) -> Kind:
    """The kind of the MIME type whose essence is `essence`, lowercase."""
    if essence == "text/html":
        return "html"
    if essence in {
        "application/xhtml+xml",
        "text/xml",
        "application/xml",
    } or essence.endswith("+xml"):
        return "xml"
    return "other"


def declared(body: bytes, kind: Kind) -> str | None:
    """The encoding `body`, a document of `kind`, declares in its own bytes.

    For HTML, a `<meta>` the prescan finds, else its XML declaration; for an
    XML type, its XML declaration; none for any other type.
    """
    if kind == "html":
        found = prescan(body)
        return found if found is not None else xml_declaration(body)
    if kind == "xml":
        return xml_declaration(body)
    return None


def _byte_based(encoding: str) -> str:
    """`encoding`, or UTF-8 for UTF-16, as Blink maps a label a document declares itself."""
    return UTF_8 if encoding in {UTF_16LE, UTF_16BE} else encoding


def xml_declaration(body: bytes) -> str | None:
    """The encoding an XML declaration at the very start of `body` names.

    As Blink's `TextResourceDecoder` reads it: within `<?xml … >`, the first
    `encoding`, then `=` and a quoted label, spaces allowed between.
    """
    if len(body) < 8 or not body.startswith(b"<?xml"):
        return None
    end = body.find(b">")
    if end == -1:
        return None
    declaration = body[:end]
    at = declaration.find(b"encoding")
    if at == -1:
        return None
    rest = _skip_spaces(declaration[at + len(b"encoding") :])
    if not rest.startswith(b"="):
        return None
    rest = _skip_spaces(rest[1:])
    if not rest or rest[0] not in b"\"'":
        return None
    close = rest.find(rest[:1], 1)
    if close == -1:
        return None
    named = for_label(rest[1:close].decode("latin-1"))
    return None if named is None else _byte_based(named)


def _skip_spaces(rest: bytes) -> bytes:
    """`rest` past its first bytes up to the space, each of which Blink skips as a space."""
    skipped = 0
    while skipped < len(rest) and rest[skipped] <= 0x20:
        skipped += 1
    return rest[skipped:]


def prescan(body: bytes) -> str | None:
    """The encoding a `<meta>` of `body` declares, read as Chrome's prescan reads it.

    Blink's `HTMLMetaCharsetParser`: the bytes tokenized as the HTML
    standard's tokenizer does, each a Latin-1 character, the text of a
    `script`, `style`, `title` and the like read as text; the first `<meta>`
    whose `charset`, or whose `content` beside an
    `http-equiv="content-type"`, names a known label wins; the scan stops
    once a tag that is not allowed in the head appeared and 1,024 bytes were
    read, checked as each token ends.
    """
    scan = _Prescan()
    for consumed, byte in enumerate(body):
        # Blink's tokenizer ends a run of text when a tag opens.
        if byte == _LESS_THAN:
            text_ended, scan.text = scan.text, False
            if text_ended and scan.done(consumed):
                return None
        scan.feed(byte)
        if scan.found is not None:
            return scan.found
        ended, scan.ended = scan.ended, False
        if ended and scan.done(consumed + 1):
            return None
    return None


_LESS_THAN: Final = 0x3C
_SPACES: Final = frozenset(b"\t\n\x0c\r ")
_HTML_SPACES: Final = "\t\n\x0c\r "
_ALLOWED_IN_HEAD: Final = frozenset(
    {"script", "noscript", "style", "link", "meta", "object", "title", "base"}
)
_RAW_TEXT: Final = {
    "title": "rcdata",
    "textarea": "rcdata",
    "script": "script",
    "style": "rawtext",
    "iframe": "rawtext",
    "xmp": "rawtext",
    "noembed": "rawtext",
    "noframes": "rawtext",
    "plaintext": "plaintext",
}
"""The text state each start tag switches to: Blink's `UpdateStateFor`, scripting off."""


def _is_alpha(byte: int) -> bool:
    return 0x41 <= byte <= 0x5A or 0x61 <= byte <= 0x7A


def _lower(byte: int) -> str:
    return chr(byte + 0x20) if 0x41 <= byte <= 0x5A else chr(byte)


class _Prescan:
    """The HTML standard's tokenizer, fed one byte at a time, and what the prescan saw."""

    def __init__(self) -> None:
        """Starts in the data state, having seen nothing."""
        self.found: str | None = None
        """The encoding a `<meta>` declared."""
        self.left_head = False
        """Whether a tag not allowed in the head appeared."""
        self.ended = False
        """Whether a token other than text ended in the last byte fed."""
        self.text = False
        """Whether a run of text is open."""
        self._state = "data"
        self._raw = "data"
        self._start = False
        self._name = ""
        self._attributes: list[tuple[str, str]] = []
        self._attribute: tuple[str, str] | None = None
        self._last_start = ""
        self._buffer = ""

    def done(self, consumed: int) -> bool:
        """Whether the scan stops, `consumed` bytes read."""
        return self.left_head and consumed >= UNCONDITIONAL

    def feed(self, byte: int) -> None:
        """Tokenizes `byte`, in each state it is reconsumed in."""
        while not self._step(byte):
            pass

    def _step(self, byte: int) -> bool:  # noqa: C901, PLR0911, PLR0912 - one branch per tokenizer state
        """Tokenizes `byte` in the current state; `False` when it is reconsumed in the next one."""
        state = self._state
        if state == "data":
            if byte == _LESS_THAN:
                self._state = "tag_open"
            else:
                self.text = True
            return True
        if state in {"rcdata", "rawtext", "script"}:
            if byte == _LESS_THAN:
                self._state = "raw_less_than"
            else:
                self.text = True
            return True
        if state == "plaintext":
            self.text = True
            return True
        if state.startswith("raw_"):
            return self._raw_end_tag(byte)
        if state in {"tag_open", "end_tag_open", "markup_declaration"}:
            return self._open(byte)
        if state.startswith("comment") or state in {"doctype", "bogus_comment"}:
            return self._comment(byte)
        return self._tag(byte)

    def _open(self, byte: int) -> bool:
        """`<` read: a tag, an end tag, a comment, a doctype or text."""
        state = self._state
        if state == "tag_open":
            if byte == 0x21:  # !
                self._state, self._buffer = "markup_declaration", ""
            elif byte == 0x2F:  # /
                self._state = "end_tag_open"
            elif _is_alpha(byte):
                self._begin_tag(start=True)
                return False
            elif byte == 0x3F:  # ?
                self._state = "bogus_comment"
                return False
            else:
                self.text = True
                self._state = "data"
                return False
            return True
        if state == "end_tag_open":
            if _is_alpha(byte):
                self._begin_tag(start=False)
                return False
            if byte == 0x3E:  # >
                self._state = "data"
                return True
            self._state = "bogus_comment"
            return False
        self._buffer += _lower(byte)
        if self._buffer == "--":
            self._state = "comment_start"
        elif self._buffer == "doctype":
            self._state = "doctype"
        elif not ("--".startswith(self._buffer) or "doctype".startswith(self._buffer)):
            self._state = "bogus_comment"
            return False
        return True

    def _comment(self, byte: int) -> bool:  # noqa: PLR0911 - one branch per comment state
        """A comment, a bogus comment or a doctype, which ends at its `>`."""
        state = self._state
        dash, closing = byte == 0x2D, byte == 0x3E
        if state in {"doctype", "bogus_comment"}:
            if closing:
                self._end_token()
            return True
        if state in {"comment_start", "comment_start_dash"}:
            if dash:
                self._state = (
                    "comment_start_dash" if state == "comment_start" else "comment_end"
                )
                return True
            if closing:
                self._end_token()
                return True
            self._state = "comment"
            return False
        if state == "comment":
            if dash:
                self._state = "comment_end_dash"
            return True
        if state == "comment_end_dash":
            if dash:
                self._state = "comment_end"
                return True
            self._state = "comment"
            return False
        if state == "comment_end":
            if closing:
                self._end_token()
            elif byte == 0x21:  # !
                self._state = "comment_end_bang"
            elif not dash:
                self._state = "comment"
                return False
            return True
        # comment_end_bang
        if dash:
            self._state = "comment_end_dash"
            return True
        if closing:
            self._end_token()
            return True
        self._state = "comment"
        return False

    def _raw_end_tag(self, byte: int) -> bool:
        """`<` read in a text state: its appropriate end tag, or text."""
        state = self._state
        if state == "raw_less_than":
            if byte == 0x2F:  # /
                self._state, self._buffer = "raw_end_tag_open", ""
                return True
            self.text = True
            self._state = self._raw
            return False
        if state == "raw_end_tag_open":
            if _is_alpha(byte):
                self._begin_tag(start=False)
                self._state = "raw_end_tag_name"
                return False
            self.text = True
            self._state = self._raw
            return False
        appropriate = self._name == self._last_start
        if byte in _SPACES and appropriate:
            self._state = "before_attribute_name"
        elif byte == 0x2F and appropriate:
            self._state = "self_closing"
        elif byte == 0x3E and appropriate:
            self._emit_tag()
        elif _is_alpha(byte):
            self._name += _lower(byte)
        else:
            self.text = True
            self._state = self._raw
            return False
        return True

    def _tag(self, byte: int) -> bool:  # noqa: C901, PLR0911, PLR0912 - one branch per tag state
        """A tag's name and attributes, up to its `>`."""
        state = self._state
        space, closing = byte in _SPACES, byte == 0x3E
        if state == "tag_name":
            if space:
                self._state = "before_attribute_name"
            elif byte == 0x2F:
                self._state = "self_closing"
            elif closing:
                self._emit_tag()
            else:
                self._name += _lower(byte)
            return True
        if state == "before_attribute_name":
            if space:
                return True
            if byte in {0x2F, 0x3E}:
                self._state = "after_attribute_name"
                return False
            self._begin_attribute()
            if byte == 0x3D:  # =
                self._append_name(byte)
                return True
            return False
        if state == "attribute_name":
            if space or byte in {0x2F, 0x3E}:
                self._state = "after_attribute_name"
                return False
            if byte == 0x3D:
                self._state = "before_attribute_value"
            else:
                self._append_name(byte)
            return True
        if state == "after_attribute_name":
            if space:
                return True
            if byte == 0x2F:
                self._state = "self_closing"
            elif byte == 0x3D:
                self._state = "before_attribute_value"
            elif closing:
                self._emit_tag()
            else:
                self._begin_attribute()
                return False
            return True
        if state == "before_attribute_value":
            if space:
                return True
            if byte == 0x22:
                self._state = "attribute_value_double"
            elif byte == 0x27:
                self._state = "attribute_value_single"
            elif closing:
                self._emit_tag()
            else:
                self._state = "attribute_value_unquoted"
                return False
            return True
        if state in {"attribute_value_double", "attribute_value_single"}:
            if byte == (0x22 if state == "attribute_value_double" else 0x27):
                self._state = "after_attribute_value"
            else:
                self._append_value(byte)
            return True
        if state == "attribute_value_unquoted":
            if space:
                self._state = "before_attribute_name"
            elif closing:
                self._emit_tag()
            else:
                self._append_value(byte)
            return True
        if state == "after_attribute_value":
            if space:
                self._state = "before_attribute_name"
            elif byte == 0x2F:
                self._state = "self_closing"
            elif closing:
                self._emit_tag()
            else:
                self._state = "before_attribute_name"
                return False
            return True
        # self_closing
        if closing:
            self._emit_tag()
            return True
        self._state = "before_attribute_name"
        return False

    def _begin_tag(self, *, start: bool) -> None:
        self._state, self._start, self._name = "tag_name", start, ""
        self._attributes, self._attribute = [], None

    def _begin_attribute(self) -> None:
        self._finish_attribute()
        self._attribute = ("", "")
        self._state = "attribute_name"

    def _append_name(self, byte: int) -> None:
        name, value = self._attribute or ("", "")
        self._attribute = (name + _lower(byte), value)

    def _append_value(self, byte: int) -> None:
        name, value = self._attribute or ("", "")
        self._attribute = (name, value + chr(byte))

    def _finish_attribute(self) -> None:
        """Keeps the attribute read, unless the tag has one of its name: the first wins."""
        if self._attribute is not None:
            name = self._attribute[0]
            if all(name != kept for kept, _ in self._attributes):
                self._attributes.append(self._attribute)
            self._attribute = None

    def _end_token(self) -> None:
        self.ended = True
        self._state = "data"

    def _emit_tag(self) -> None:
        """A tag ended: what it says of the head, its `<meta>`'s encoding, and the state it opens."""
        self._finish_attribute()
        self._end_token()
        name = self._name
        allowed = name in _ALLOWED_IN_HEAD or (self._start and name in {"html", "head"})
        if not allowed:
            self.left_head = True
        if not self._start:
            return
        self._last_start = name
        if name == "meta":
            encoding = _meta_encoding(self._attributes)
            if encoding is not None:
                self.found = encoding
        self._raw = _RAW_TEXT.get(name, "data")
        self._state = self._raw


def _meta_encoding(attributes: list[tuple[str, str]]) -> str | None:
    """The encoding a `<meta>` of `attributes` declares (Blink's `EncodingFromMetaAttributes`)."""
    pragma = False
    charset: tuple[str, bool] | None = None
    for name, value in attributes:
        if name == "http-equiv":
            pragma |= value.translate(_ASCII_LOWER) == "content-type"
        elif name == "charset":
            charset = (value, True)
        elif name == "content" and not (charset is not None and charset[1]):
            extracted = _extract_charset(value)
            if extracted:
                charset = (extracted, False)
    if charset is None:
        return None
    label, from_charset = charset
    if not from_charset and not pragma:
        return None
    encoding = for_label(label.strip(_HTML_SPACES))
    if encoding is None:
        return None
    return WINDOWS_1252 if encoding == X_USER_DEFINED else _byte_based(encoding)


def _extract_charset(value: str) -> str:  # noqa: C901 - the standard's algorithm, step by step
    """The label a `<meta>`'s `content` names after `charset=`; empty for none.

    The HTML standard's algorithm, as Blink's `ExtractCharset` runs it.
    """
    lowered = value.translate(_ASCII_LOWER)
    length = len(value)
    start = 0
    while (at := lowered.find("charset", start)) != -1:
        position = at + len("charset")
        while position < length and value[position] <= " ":
            position += 1
        if position >= length or value[position] != "=":
            start = position
            continue
        position += 1
        while position < length and value[position] <= " ":
            position += 1
        quote = (
            value[position] if position < length and value[position] in "\"'" else None
        )
        if quote is not None:
            position += 1
        if position >= length:
            break
        end = position
        while end < length and (
            value[end] != quote
            if quote is not None
            else value[end] > " " and value[end] not in "\"';"
        ):
            end += 1
        if quote is not None and end == length:
            break
        return value[position:end]
    return ""
