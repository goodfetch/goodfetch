// A response's body as text and as JSON (ADR 0119), as Rust's client reads
// it. `text()` decodes with the response's own encoding: a navigation's
// document with the one its document was decoded with, any other with its
// BOM, its `Content-Type` charset, Chrome's `<meta>` prescan for HTML or the
// XML declaration for an XML type, then UTF-8; `json()` decodes UTF-8 and
// ignores a BOM. Rust's `response/tests.rs` and `goodfetch_encoding`'s tests
// are its twins, and Python's `test_decoding.py`.
import { describe, expect, test } from "vitest";
import {
  decode,
  jsonOf,
  kindOf,
  mediaType,
  prescan,
  text,
  xmlDeclaration,
  type Kind,
} from "./decoding.ts";

/** 日本語 in `Shift_JIS`. */
const SHIFT_JIS_BYTES = [0x93, 0xfa, 0x96, 0x7b, 0x8c, 0xea];

/** 日本 in `Shift_JIS`, which is no UTF-8. */
const NIHON = [0x93, 0xfa, 0x96, 0x7b];

/** `é` in windows-1252. */
const E_ACUTE = 0xe9;

const UTF_8 = "UTF-8";
const SHIFT_JIS = "Shift_JIS";
const WINDOWS_1251 = "windows-1251";
const WINDOWS_1252 = "windows-1252";

/** The bytes of `parts`: a string's in UTF-8, a number's as one byte, an array's as its bytes. */
function bytes(...parts: readonly (string | number | readonly number[])[]): Uint8Array {
  const encoder = new TextEncoder();
  return Uint8Array.from(
    parts.flatMap((part) =>
      typeof part === "string"
        ? [...encoder.encode(part)]
        : typeof part === "number"
          ? [part]
          : [...part],
    ),
  );
}

/** `line` repeated past the first 1,024 bytes of a document. */
function past1024(line: string): string {
  return line.repeat(Math.floor(1024 / line.length) + 2);
}

/** An HTML document holding `head` and `body`, with the `Shift_JIS` text 日本 in its body. */
function html(head: string, body: string): Uint8Array {
  return bytes(
    `<!doctype html><html><head>${head}</head><body>${body}<p>`,
    NIHON,
    "</p></body></html>",
  );
}

describe("text", () => {
  test(
    "decodes a navigation's document with the encoding its document was decoded with",
    { tags: ["story:11-85"] },
    () => {
      const body = bytes("<p>", SHIFT_JIS_BYTES, "</p>");

      expect(text(body, "Shift_JIS", "text/html; charset=utf-8")).toBe("<p>日本語</p>");
    },
  );

  test(
    "takes a BOM over the Content-Type charset and leaves it out",
    { tags: ["story:11-85"] },
    () => {
      const body = bytes([0xef, 0xbb, 0xbf], "café");

      expect(text(body, null, "text/plain; charset=windows-1252")).toBe("café");
    },
  );

  test.for([
    "text/plain; charset=Shift_JIS",
    'text/plain; charset="shift_jis"',
    "text/plain; format=flowed; Charset=sjis",
  ])("decodes a body with the legacy charset of %s", { tags: ["story:11-85"] }, (contentType) => {
    expect(text(bytes(SHIFT_JIS_BYTES), null, contentType)).toBe("日本語");
  });

  test.for([
    ["a charset attribute", '<!doctype html><html><head><meta charset="shift_jis"><title>'],
    [
      "a pragma",
      '<html><head><meta http-equiv="Content-Type" content="text/html; charset=Shift_JIS"><title>',
    ],
    [
      "a late one among head tags",
      `<html><head>${"<meta name=k content=v>".repeat(50)}<meta charset=sjis><title>`,
    ],
  ] as const)("decodes an HTML body with its <meta>: %s", { tags: ["story:11-85"] }, ([, head]) => {
    const body = bytes(head, SHIFT_JIS_BYTES, "</title>");

    expect(text(body, null, "text/html")).toBe(`${head}日本語</title>`);
  });

  test(
    "reads a Content-Type charset no encoding has as none, giving way to the <meta>",
    { tags: ["story:11-85"] },
    () => {
      const head = '<meta charset="windows-1252"><p>';
      const body = bytes(head, E_ACUTE, "</p>");

      expect(text(body, null, "text/html; charset=no-such-encoding")).toBe(`${head}é</p>`);
    },
  );

  test.for(["application/xml", "text/xml", "application/atom+xml", "text/html"])(
    "decodes a %s body with the XML declaration at its start",
    { tags: ["story:11-85"] },
    (contentType) => {
      const head = '<?xml version="1.0" encoding="windows-1252"?><a>';
      const body = bytes(head, E_ACUTE, "</a>");

      expect(text(body, null, contentType)).toBe(`${head}é</a>`);
    },
  );

  test.for([["text/plain"], [undefined]] as const)(
    "decodes a body of type %s as UTF-8, reading no declaration",
    { tags: ["story:11-85"] },
    ([contentType]) => {
      const head = '<?xml version="1.0" encoding="windows-1252"?><a>';
      const body = bytes(head, E_ACUTE, "</a>");

      expect(text(body, null, contentType)).toBe(`${head}�</a>`);
    },
  );
});

describe("decode", () => {
  test.for([
    ["a UTF-16 BOM is left out", [0xff, 0xfe, 0x61, 0x00], "UTF-16LE", "a"],
    ["a C1 control", [0x80, 0x81], "windows-1252", "€\u0081"],
    ["the standard's KOI8-U", [0xae], "KOI8-U", "ў"],
    ["x-user-defined", [0xf8], "x-user-defined", String.fromCharCode(0xf7f8)],
    ["replacement", [0x61, 0x62, 0x63], "replacement", "�"],
    ["a byte the encoding leaves undefined", [0xa1], "ISO-8859-6", "�"],
  ] as const)(
    "decodes as the Encoding Standard maps it: %s",
    { tags: ["story:11-85"] },
    ([, body, encoding, decoded]) => {
      expect(decode(bytes(body), encoding)).toBe(decoded);
    },
  );
});

describe("jsonOf", () => {
  test(
    "ignores a leading BOM and decodes UTF-8 whatever the charset",
    { tags: ["story:11-85"] },
    () => {
      const body = bytes([0xef, 0xbb, 0xbf], '{"name":"café"}');

      expect(jsonOf(body)).toEqual({ name: "café" });
    },
  );

  test.for(["<html></html>", "[NaN]"])(
    "fails on %s, which is no JSON",
    { tags: ["story:11-85"] },
    (body) => {
      expect(() => jsonOf(bytes(body))).toThrow(SyntaxError);
    },
  );
});

describe("mediaType", () => {
  test.for([
    ["text/html; charset=Shift_JIS", "text/html", "Shift_JIS"],
    ['Text/HTML;Charset="shift_jis"', "text/html", "shift_jis"],
    ["application/xhtml+xml; q=1 ; charset = utf-8", "application/xhtml+xml", "utf-8"],
    [" text/plain ", "text/plain", undefined],
  ] as const)(
    "reads %s as its essence and charset",
    { tags: ["story:6-233"] },
    ([value, essence, charset]) => {
      expect(mediaType(value)).toEqual([essence, charset]);
    },
  );
});

describe("kindOf", () => {
  test.for([
    ["text/html", "html"],
    ["application/xhtml+xml", "xml"],
    ["text/xml", "xml"],
    ["image/svg+xml", "xml"],
    ["application/json", "other"],
  ] as const satisfies readonly (readonly [string, Kind])[])(
    "reads %s as a body of kind %s",
    { tags: ["story:5-129"] },
    ([essence, kind]) => {
      expect(kindOf(essence)).toBe(kind);
    },
  );
});

describe("prescan", () => {
  // Blink's `HTMLMetaCharsetParser`: 1,024 bytes always, then on while only
  // head tags appear. UTF-16 reads as UTF-8 and `x-user-defined` as
  // windows-1252; a declaration in a comment, a title or a script is none,
  // and an unknown label is skipped.
  test.for([
    ["a charset", html("<meta charset=shift_jis>", ""), SHIFT_JIS],
    ["a quoted charset in any case", html("<META CharSet='Shift_JIS'>", ""), SHIFT_JIS],
    [
      "a pragma",
      html("<meta content=\"text/html;charset='shift_jis'\" http-equiv=content-type>", ""),
      SHIFT_JIS,
    ],
    ["a content alone", html('<meta content="text/html; charset=shift_jis">', ""), undefined],
    ["a UTF-16 label", html("<meta charset=utf-16le>", ""), UTF_8],
    ["x-user-defined", html("<meta charset=x-user-defined>", ""), WINDOWS_1252],
    [
      "an unknown label then a known one",
      html("<meta charset=klingon><meta charset=windows-1251>", ""),
      WINDOWS_1251,
    ],
    ["in a comment", html("<!-- <meta charset=shift_jis> -->", ""), undefined],
    ["in a title", html("<title><meta charset=shift_jis></title>", ""), undefined],
    ["in a script", html("<script>'<meta charset=shift_jis>'</script>", ""), undefined],
    [
      "late among head tags",
      html(`${past1024("<meta name=k content=v>")}<meta charset=windows-1251>`, ""),
      WINDOWS_1251,
    ],
    [
      "after the body in the first 1024 bytes",
      html("", "<meta charset=windows-1251>"),
      WINDOWS_1251,
    ],
    [
      "after the body past 1024 bytes",
      html("", `${past1024("<p>filler</p>")}<meta charset=windows-1251>`),
      undefined,
    ],
    ["an unclosed tag", bytes("<meta charset=shift_jis"), undefined],
  ] as const)(
    "reads a <meta> as Chrome does: %s",
    { tags: ["story:5-129"] },
    ([, body, expected]) => {
      expect(prescan(body)).toBe(expected);
    },
  );
});

describe("xmlDeclaration", () => {
  // Blink's `TextResourceDecoder`: at the very start, its `encoding` quoted,
  // UTF-16 as UTF-8.
  test.for([
    ["double quoted", '<?xml version="1.0" encoding="Shift_JIS"?><a/>', SHIFT_JIS],
    ["single quoted, spaced", "<?xml version='1.0' encoding = 'windows-1251' ?>", WINDOWS_1251],
    ["UTF-16", '<?xml version="1.0" encoding="UTF-16"?>', UTF_8],
    ["no encoding", '<?xml version="1.0"?><a/>', undefined],
    ["unquoted", '<?xml version="1.0" encoding=Shift_JIS?>', undefined],
    ["not at the start", ' <?xml version="1.0" encoding="Shift_JIS"?>', undefined],
    ["unclosed", '<?xml version="1.0" encoding="Shift_JIS"', undefined],
  ] as const)("names the encoding: %s", { tags: ["story:5-129"] }, ([, body, expected]) => {
    expect(xmlDeclaration(bytes(body))).toBe(expected);
  });
});
