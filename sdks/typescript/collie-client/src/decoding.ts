// A response body's text and JSON (ADR 0119), as Rust's client reads them.
// A navigation's document decodes with the encoding its document was
// decoded with, the browser's `document.characterSet`; any other response
// with its BOM, its `Content-Type` charset, the encoding it declares in its
// own bytes (for HTML Chrome's `<meta>` prescan then an XML declaration at
// the very start, for an XML type that declaration), and UTF-8 last. A byte
// that does not decode becomes U+FFFD. JSON is UTF-8 whatever the charset,
// a leading BOM ignored. Rust's `goodfetch_encoding` crate is the reading
// this module follows; Python's `_decoding.py` is its twin.
//
// The labels are the Encoding Standard's, as `encoding_rs` 0.8.44 holds
// them; the platform's `TextDecoder` decodes, but for `replacement`, which
// Node refuses, and `x-user-defined`, which it decodes otherwise. The
// prescan reads the bytes with the HTML standard's tokenizer states written
// here, where Rust's runs html5ever's: a character reference is read as its
// own characters, and a script's escaped text as plain script text, which
// no `<meta>` read depends on.

import type { JSONValue } from "./json.ts";

/** How many bytes the prescan always reads, as Blink's `kBytesToCheckUnconditionally`. */
export const UNCONDITIONAL = 1024;

const UTF_8 = "UTF-8";
const UTF_16LE = "UTF-16LE";
const UTF_16BE = "UTF-16BE";
const WINDOWS_1252 = "windows-1252";
const X_USER_DEFINED = "x-user-defined";
const REPLACEMENT = "replacement";

/** Each encoding, by its Encoding Standard name, and its labels. */
const ENCODINGS: Readonly<Record<string, readonly string[]>> = {
  Big5: ["big5", "big5-hkscs", "cn-big5", "csbig5", "x-x-big5"],
  "EUC-JP": ["cseucpkdfmtjapanese", "euc-jp", "x-euc-jp"],
  "EUC-KR": [
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
  ],
  gb18030: ["gb18030"],
  GBK: [
    "chinese",
    "csgb2312",
    "csiso58gb231280",
    "gb2312",
    "gb_2312",
    "gb_2312-80",
    "gbk",
    "iso-ir-58",
    "x-gbk",
  ],
  IBM866: ["866", "cp866", "csibm866", "ibm866"],
  "ISO-2022-JP": ["csiso2022jp", "iso-2022-jp"],
  "ISO-8859-10": [
    "csisolatin6",
    "iso-8859-10",
    "iso-ir-157",
    "iso8859-10",
    "iso885910",
    "l6",
    "latin6",
  ],
  "ISO-8859-13": ["iso-8859-13", "iso8859-13", "iso885913"],
  "ISO-8859-14": ["iso-8859-14", "iso8859-14", "iso885914"],
  "ISO-8859-15": ["csisolatin9", "iso-8859-15", "iso8859-15", "iso885915", "iso_8859-15", "l9"],
  "ISO-8859-16": ["iso-8859-16"],
  "ISO-8859-2": [
    "csisolatin2",
    "iso-8859-2",
    "iso-ir-101",
    "iso8859-2",
    "iso88592",
    "iso_8859-2",
    "iso_8859-2:1987",
    "l2",
    "latin2",
  ],
  "ISO-8859-3": [
    "csisolatin3",
    "iso-8859-3",
    "iso-ir-109",
    "iso8859-3",
    "iso88593",
    "iso_8859-3",
    "iso_8859-3:1988",
    "l3",
    "latin3",
  ],
  "ISO-8859-4": [
    "csisolatin4",
    "iso-8859-4",
    "iso-ir-110",
    "iso8859-4",
    "iso88594",
    "iso_8859-4",
    "iso_8859-4:1988",
    "l4",
    "latin4",
  ],
  "ISO-8859-5": [
    "csisolatincyrillic",
    "cyrillic",
    "iso-8859-5",
    "iso-ir-144",
    "iso8859-5",
    "iso88595",
    "iso_8859-5",
    "iso_8859-5:1988",
  ],
  "ISO-8859-6": [
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
  ],
  "ISO-8859-7": [
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
  ],
  "ISO-8859-8": [
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
  ],
  "ISO-8859-8-I": ["csiso88598i", "iso-8859-8-i", "logical"],
  "KOI8-R": ["cskoi8r", "koi", "koi8", "koi8-r", "koi8_r"],
  "KOI8-U": ["koi8-ru", "koi8-u"],
  macintosh: ["csmacintosh", "mac", "macintosh", "x-mac-roman"],
  [REPLACEMENT]: [
    "csiso2022kr",
    "hz-gb-2312",
    "iso-2022-cn",
    "iso-2022-cn-ext",
    "iso-2022-kr",
    "replacement",
  ],
  Shift_JIS: [
    "csshiftjis",
    "ms932",
    "ms_kanji",
    "shift-jis",
    "shift_jis",
    "sjis",
    "windows-31j",
    "x-sjis",
  ],
  [UTF_16BE]: ["unicodefffe", "utf-16be"],
  [UTF_16LE]: [
    "csunicode",
    "iso-10646-ucs-2",
    "ucs-2",
    "unicode",
    "unicodefeff",
    "utf-16",
    "utf-16le",
  ],
  [UTF_8]: [
    "unicode-1-1-utf-8",
    "unicode11utf8",
    "unicode20utf8",
    "utf-8",
    "utf8",
    "x-unicode20utf8",
  ],
  "windows-1250": ["cp1250", "windows-1250", "x-cp1250"],
  "windows-1251": ["cp1251", "windows-1251", "x-cp1251"],
  [WINDOWS_1252]: [
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
  ],
  "windows-1253": ["cp1253", "windows-1253", "x-cp1253"],
  "windows-1254": [
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
  ],
  "windows-1255": ["cp1255", "windows-1255", "x-cp1255"],
  "windows-1256": ["cp1256", "windows-1256", "x-cp1256"],
  "windows-1257": ["cp1257", "windows-1257", "x-cp1257"],
  "windows-1258": ["cp1258", "windows-1258", "x-cp1258"],
  "windows-874": ["dos-874", "iso-8859-11", "iso8859-11", "iso885911", "tis-620", "windows-874"],
  "x-mac-cyrillic": ["x-mac-cyrillic", "x-mac-ukrainian"],
  [X_USER_DEFINED]: ["x-user-defined"],
};

const LABELS: ReadonlyMap<string, string> = new Map(
  Object.entries(ENCODINGS).flatMap(([name, labels]) => labels.map((label) => [label, name])),
);

const BOMS: readonly (readonly [readonly number[], string])[] = [
  [[0xef, 0xbb, 0xbf], UTF_8],
  [[0xff, 0xfe], UTF_16LE],
  [[0xfe, 0xff], UTF_16BE],
];

/** Which declaration in its own bytes a document's type reads. */
export type Kind = "html" | "xml" | "other";

/** `value` lowercase in ASCII only, as the Encoding Standard and MIME types compare. */
function asciiLower(value: string): string {
  return value.replace(/[A-Z]/g, (letter) => letter.toLowerCase());
}

/**
 * `body` as text: decoded with `document`, the label of the encoding a
 * navigation's document was decoded with, when it names one; otherwise
 * with the encoding `body` and its `Content-Type` value `contentType`
 * declare. A BOM is left out, and a byte that does not decode becomes
 * U+FFFD.
 */
export function text(
  body: Uint8Array,
  document: string | null | undefined,
  contentType: string | undefined,
): string {
  const named = document === null || document === undefined ? undefined : forLabel(document);
  return decode(body, named ?? ofBody(body, contentType));
}

/**
 * `body` as JSON, decoded as UTF-8 whatever its charset, a leading BOM
 * ignored. Throws a `SyntaxError` for a body that is not JSON.
 */
export function jsonOf(body: Uint8Array): JSONValue {
  const parsed: JSONValue = JSON.parse(decode(body, UTF_8));
  return parsed;
}

/** The encoding `label` names, as the Encoding Standard's `get an encoding` reads it. */
export function forLabel(label: string): string | undefined {
  return LABELS.get(asciiLower(label.replace(/^[\t\n\f\r ]+|[\t\n\f\r ]+$/g, "")));
}

function startsWith(body: Uint8Array, prefix: readonly number[]): boolean {
  return body.length >= prefix.length && prefix.every((byte, at) => body[at] === byte);
}

/** The encoding `body`, whose `Content-Type` value is `contentType`, declares; UTF-8 when none. */
export function ofBody(body: Uint8Array, contentType: string | undefined): string {
  const bom = BOMS.find(([bytes]) => startsWith(body, bytes));
  if (bom !== undefined) {
    return bom[1];
  }
  const [essence, charset] = contentType === undefined ? ["", undefined] : mediaType(contentType);
  const named = charset === undefined ? undefined : forLabel(charset);
  return named ?? declared(body, kindOf(essence)) ?? UTF_8;
}

/** `body` decoded with `encoding`, its own BOM left out, a byte that does not decode U+FFFD. */
export function decode(body: Uint8Array, encoding: string): string {
  if (encoding === REPLACEMENT) {
    return body.length === 0 ? "" : "�";
  }
  if (encoding === X_USER_DEFINED) {
    let decoded = "";
    for (const byte of body) {
      decoded += String.fromCharCode(byte < 0x80 ? byte : 0xf700 + byte);
    }
    return decoded;
  }
  return new TextDecoder(encoding).decode(body);
}

/** The essence of the `Content-Type` value `value`, lowercase, and its `charset`, unquoted. */
export function mediaType(value: string): readonly [string, string | undefined] {
  const [essence = "", ...parameters] = value.split(";");
  const lowered = asciiLower(essence.trim());
  for (const parameter of parameters) {
    const equals = parameter.indexOf("=");
    if (equals !== -1 && asciiLower(parameter.slice(0, equals).trim()) === "charset") {
      return [
        lowered,
        parameter
          .slice(equals + 1)
          .trim()
          .replace(/^"+|"+$/g, ""),
      ];
    }
  }
  return [lowered, undefined];
}

/** The kind of the MIME type whose essence is `essence`, lowercase. */
export function kindOf(essence: string): Kind {
  if (essence === "text/html") {
    return "html";
  }
  const xml = ["application/xhtml+xml", "text/xml", "application/xml"];
  return xml.includes(essence) || essence.endsWith("+xml") ? "xml" : "other";
}

/**
 * The encoding `body`, a document of `kind`, declares in its own bytes:
 * for HTML, a `<meta>` the prescan finds, else its XML declaration; for an
 * XML type, its XML declaration; none for any other type.
 */
export function declared(body: Uint8Array, kind: Kind): string | undefined {
  switch (kind) {
    case "html":
      return prescan(body) ?? xmlDeclaration(body);
    case "xml":
      return xmlDeclaration(body);
    case "other":
      return undefined;
    default: {
      const unknown: never = kind;
      return unknown;
    }
  }
}

/** `encoding`, or UTF-8 for UTF-16, as Blink maps a label a document declares itself. */
function byteBased(encoding: string): string {
  return encoding === UTF_16LE || encoding === UTF_16BE ? UTF_8 : encoding;
}

/** `bytes` as Latin-1 characters, one a byte. */
function latin1(bytes: Uint8Array): string {
  let characters = "";
  for (const byte of bytes) {
    characters += String.fromCharCode(byte);
  }
  return characters;
}

/**
 * The encoding an XML declaration at the very start of `body` names, as
 * Blink's `TextResourceDecoder` reads it: within `<?xml … >`, the first
 * `encoding`, then `=` and a quoted label, spaces allowed between.
 */
export function xmlDeclaration(body: Uint8Array): string | undefined {
  if (body.length < 8 || !startsWith(body, [0x3c, 0x3f, 0x78, 0x6d, 0x6c])) {
    return undefined;
  }
  const end = body.indexOf(0x3e);
  if (end === -1) {
    return undefined;
  }
  const declaration = latin1(body.subarray(0, end));
  const at = declaration.indexOf("encoding");
  if (at === -1) {
    return undefined;
  }
  let rest = skipSpaces(declaration.slice(at + "encoding".length));
  if (!rest.startsWith("=")) {
    return undefined;
  }
  rest = skipSpaces(rest.slice(1));
  const quote = rest[0];
  if (quote !== '"' && quote !== "'") {
    return undefined;
  }
  const close = rest.indexOf(quote, 1);
  if (close === -1) {
    return undefined;
  }
  const named = forLabel(rest.slice(1, close));
  return named === undefined ? undefined : byteBased(named);
}

/** `rest` past its first characters up to the space, each of which Blink skips as a space. */
function skipSpaces(rest: string): string {
  let skipped = 0;
  while (skipped < rest.length && rest.charCodeAt(skipped) <= 0x20) {
    skipped += 1;
  }
  return rest.slice(skipped);
}

/**
 * The encoding a `<meta>` of `body` declares, read as Chrome's prescan
 * reads it (Blink's `HTMLMetaCharsetParser`): the bytes tokenized as the
 * HTML standard's tokenizer does, each a Latin-1 character, the text of a
 * `script`, `style`, `title` and the like read as text; the first `<meta>`
 * whose `charset`, or whose `content` beside an
 * `http-equiv="content-type"`, names a known label wins; the scan stops
 * once a tag that is not allowed in the head appeared and 1,024 bytes were
 * read, checked as each token ends.
 */
export function prescan(body: Uint8Array): string | undefined {
  const scan = new Prescan();
  for (const [consumed, byte] of body.entries()) {
    // Blink's tokenizer ends a run of text when a tag opens.
    if (byte === LESS_THAN) {
      const textEnded = scan.text;
      scan.text = false;
      if (textEnded && scan.done(consumed)) {
        return undefined;
      }
    }
    scan.feed(byte);
    if (scan.found !== undefined) {
      return scan.found;
    }
    const ended = scan.ended;
    scan.ended = false;
    if (ended && scan.done(consumed + 1)) {
      return undefined;
    }
  }
  return undefined;
}

const LESS_THAN = 0x3c;
const SLASH = 0x2f;
const EQUALS = 0x3d;
const GREATER_THAN = 0x3e;
const DASH = 0x2d;
const BANG = 0x21;
const QUESTION = 0x3f;
const DOUBLE_QUOTE = 0x22;
const SINGLE_QUOTE = 0x27;
const HTML_SPACES = /^[\t\n\f\r ]+|[\t\n\f\r ]+$/g;
const ALLOWED_IN_HEAD: ReadonlySet<string> = new Set([
  "script",
  "noscript",
  "style",
  "link",
  "meta",
  "object",
  "title",
  "base",
]);

type TextState = "data" | "rcdata" | "rawtext" | "script" | "plaintext";

/** The text state each start tag switches to: Blink's `UpdateStateFor`, scripting off. */
const RAW_TEXT: ReadonlyMap<string, TextState> = new Map([
  ["title", "rcdata"],
  ["textarea", "rcdata"],
  ["script", "script"],
  ["style", "rawtext"],
  ["iframe", "rawtext"],
  ["xmp", "rawtext"],
  ["noembed", "rawtext"],
  ["noframes", "rawtext"],
  ["plaintext", "plaintext"],
]);

type State =
  | TextState
  | "tag_open"
  | "end_tag_open"
  | "markup_declaration"
  | "comment_start"
  | "comment_start_dash"
  | "comment"
  | "comment_end_dash"
  | "comment_end"
  | "comment_end_bang"
  | "doctype"
  | "bogus_comment"
  | "raw_less_than"
  | "raw_end_tag_open"
  | "raw_end_tag_name"
  | "tag_name"
  | "before_attribute_name"
  | "attribute_name"
  | "after_attribute_name"
  | "before_attribute_value"
  | "attribute_value_double"
  | "attribute_value_single"
  | "attribute_value_unquoted"
  | "after_attribute_value"
  | "self_closing";

function isSpace(byte: number): boolean {
  return byte === 0x09 || byte === 0x0a || byte === 0x0c || byte === 0x0d || byte === 0x20;
}

function isAlpha(byte: number): boolean {
  return (byte >= 0x41 && byte <= 0x5a) || (byte >= 0x61 && byte <= 0x7a);
}

function lower(byte: number): string {
  return String.fromCharCode(byte >= 0x41 && byte <= 0x5a ? byte + 0x20 : byte);
}

/** The HTML standard's tokenizer, fed one byte at a time, and what the prescan saw. */
class Prescan {
  /** The encoding a `<meta>` declared. */
  found: string | undefined;
  /** Whether a token other than text ended in the last byte fed. */
  ended = false;
  /** Whether a run of text is open. */
  text = false;
  /** Whether a tag not allowed in the head appeared. */
  #leftHead = false;
  #state: State = "data";
  #raw: TextState = "data";
  #start = false;
  #name = "";
  #attributes: [string, string][] = [];
  #attribute: [string, string] | undefined;
  #lastStart = "";
  #buffer = "";

  /** Whether the scan stops, `consumed` bytes read. */
  done(consumed: number): boolean {
    return this.#leftHead && consumed >= UNCONDITIONAL;
  }

  /** Tokenizes `byte`, in each state it is reconsumed in. */
  feed(byte: number): void {
    while (!this.#step(byte)) {
      // Reconsumed in the state the step switched to.
    }
  }

  /** Tokenizes `byte` in the current state; `false` when it is reconsumed in the next one. */
  #step(byte: number): boolean {
    switch (this.#state) {
      case "data":
        if (byte === LESS_THAN) {
          this.#state = "tag_open";
        } else {
          this.text = true;
        }
        return true;
      case "rcdata":
      case "rawtext":
      case "script":
        if (byte === LESS_THAN) {
          this.#state = "raw_less_than";
        } else {
          this.text = true;
        }
        return true;
      case "plaintext":
        this.text = true;
        return true;
      case "raw_less_than":
      case "raw_end_tag_open":
      case "raw_end_tag_name":
        return this.#rawEndTag(byte);
      case "tag_open":
      case "end_tag_open":
      case "markup_declaration":
        return this.#open(byte);
      case "comment_start":
      case "comment_start_dash":
      case "comment":
      case "comment_end_dash":
      case "comment_end":
      case "comment_end_bang":
      case "doctype":
      case "bogus_comment":
        return this.#comment(byte);
      default:
        return this.#tag(byte);
    }
  }

  /** `<` read: a tag, an end tag, a comment, a doctype or text. */
  #open(byte: number): boolean {
    if (this.#state === "tag_open") {
      if (byte === BANG) {
        this.#state = "markup_declaration";
        this.#buffer = "";
        return true;
      }
      if (byte === SLASH) {
        this.#state = "end_tag_open";
        return true;
      }
      if (isAlpha(byte)) {
        this.#beginTag(true);
        return false;
      }
      if (byte === QUESTION) {
        this.#state = "bogus_comment";
        return false;
      }
      this.text = true;
      this.#state = "data";
      return false;
    }
    if (this.#state === "end_tag_open") {
      if (isAlpha(byte)) {
        this.#beginTag(false);
        return false;
      }
      if (byte === GREATER_THAN) {
        this.#state = "data";
        return true;
      }
      this.#state = "bogus_comment";
      return false;
    }
    this.#buffer += lower(byte);
    if (this.#buffer === "--") {
      this.#state = "comment_start";
    } else if (this.#buffer === "doctype") {
      this.#state = "doctype";
    } else if (!"--".startsWith(this.#buffer) && !"doctype".startsWith(this.#buffer)) {
      this.#state = "bogus_comment";
      return false;
    }
    return true;
  }

  /** A comment, a bogus comment or a doctype, which ends at its `>`. */
  #comment(byte: number): boolean {
    const dash = byte === DASH;
    const closing = byte === GREATER_THAN;
    switch (this.#state) {
      case "doctype":
      case "bogus_comment":
        if (closing) {
          this.#endToken();
        }
        return true;
      case "comment_start":
      case "comment_start_dash":
        if (dash) {
          this.#state = this.#state === "comment_start" ? "comment_start_dash" : "comment_end";
          return true;
        }
        if (closing) {
          this.#endToken();
          return true;
        }
        this.#state = "comment";
        return false;
      case "comment":
        if (dash) {
          this.#state = "comment_end_dash";
        }
        return true;
      case "comment_end_dash":
        if (dash) {
          this.#state = "comment_end";
          return true;
        }
        this.#state = "comment";
        return false;
      case "comment_end":
        if (closing) {
          this.#endToken();
        } else if (byte === BANG) {
          this.#state = "comment_end_bang";
        } else if (!dash) {
          this.#state = "comment";
          return false;
        }
        return true;
      default:
        // comment_end_bang
        if (dash) {
          this.#state = "comment_end_dash";
          return true;
        }
        if (closing) {
          this.#endToken();
          return true;
        }
        this.#state = "comment";
        return false;
    }
  }

  /** `<` read in a text state: its appropriate end tag, or text. */
  #rawEndTag(byte: number): boolean {
    if (this.#state === "raw_less_than") {
      if (byte === SLASH) {
        this.#state = "raw_end_tag_open";
        return true;
      }
      this.text = true;
      this.#state = this.#raw;
      return false;
    }
    if (this.#state === "raw_end_tag_open") {
      if (isAlpha(byte)) {
        this.#beginTag(false);
        this.#state = "raw_end_tag_name";
        return false;
      }
      this.text = true;
      this.#state = this.#raw;
      return false;
    }
    const appropriate = this.#name === this.#lastStart;
    if (isSpace(byte) && appropriate) {
      this.#state = "before_attribute_name";
    } else if (byte === SLASH && appropriate) {
      this.#state = "self_closing";
    } else if (byte === GREATER_THAN && appropriate) {
      this.#emitTag();
    } else if (isAlpha(byte)) {
      this.#name += lower(byte);
    } else {
      this.text = true;
      this.#state = this.#raw;
      return false;
    }
    return true;
  }

  /** A tag's name and attributes, up to its `>`. */
  #tag(byte: number): boolean {
    const space = isSpace(byte);
    const closing = byte === GREATER_THAN;
    switch (this.#state) {
      case "tag_name":
        if (space) {
          this.#state = "before_attribute_name";
        } else if (byte === SLASH) {
          this.#state = "self_closing";
        } else if (closing) {
          this.#emitTag();
        } else {
          this.#name += lower(byte);
        }
        return true;
      case "before_attribute_name":
        if (space) {
          return true;
        }
        if (byte === SLASH || closing) {
          this.#state = "after_attribute_name";
          return false;
        }
        this.#beginAttribute();
        if (byte === EQUALS) {
          this.#appendName(byte);
          return true;
        }
        return false;
      case "attribute_name":
        if (space || byte === SLASH || closing) {
          this.#state = "after_attribute_name";
          return false;
        }
        if (byte === EQUALS) {
          this.#state = "before_attribute_value";
        } else {
          this.#appendName(byte);
        }
        return true;
      case "after_attribute_name":
        if (space) {
          return true;
        }
        if (byte === SLASH) {
          this.#state = "self_closing";
        } else if (byte === EQUALS) {
          this.#state = "before_attribute_value";
        } else if (closing) {
          this.#emitTag();
        } else {
          this.#beginAttribute();
          return false;
        }
        return true;
      case "before_attribute_value":
        if (space) {
          return true;
        }
        if (byte === DOUBLE_QUOTE) {
          this.#state = "attribute_value_double";
        } else if (byte === SINGLE_QUOTE) {
          this.#state = "attribute_value_single";
        } else if (closing) {
          this.#emitTag();
        } else {
          this.#state = "attribute_value_unquoted";
          return false;
        }
        return true;
      case "attribute_value_double":
      case "attribute_value_single":
        if (byte === (this.#state === "attribute_value_double" ? DOUBLE_QUOTE : SINGLE_QUOTE)) {
          this.#state = "after_attribute_value";
        } else {
          this.#appendValue(byte);
        }
        return true;
      case "attribute_value_unquoted":
        if (space) {
          this.#state = "before_attribute_name";
        } else if (closing) {
          this.#emitTag();
        } else {
          this.#appendValue(byte);
        }
        return true;
      case "after_attribute_value":
        if (space) {
          this.#state = "before_attribute_name";
        } else if (byte === SLASH) {
          this.#state = "self_closing";
        } else if (closing) {
          this.#emitTag();
        } else {
          this.#state = "before_attribute_name";
          return false;
        }
        return true;
      default:
        // self_closing
        if (closing) {
          this.#emitTag();
          return true;
        }
        this.#state = "before_attribute_name";
        return false;
    }
  }

  #beginTag(start: boolean): void {
    this.#state = "tag_name";
    this.#start = start;
    this.#name = "";
    this.#attributes = [];
    this.#attribute = undefined;
  }

  #beginAttribute(): void {
    this.#finishAttribute();
    this.#attribute = ["", ""];
    this.#state = "attribute_name";
  }

  #appendName(byte: number): void {
    const [name, value] = this.#attribute ?? ["", ""];
    this.#attribute = [name + lower(byte), value];
  }

  #appendValue(byte: number): void {
    const [name, value] = this.#attribute ?? ["", ""];
    this.#attribute = [name, value + String.fromCharCode(byte)];
  }

  /** Keeps the attribute read, unless the tag has one of its name: the first wins. */
  #finishAttribute(): void {
    const attribute = this.#attribute;
    if (attribute !== undefined) {
      if (this.#attributes.every(([kept]) => kept !== attribute[0])) {
        this.#attributes.push(attribute);
      }
      this.#attribute = undefined;
    }
  }

  #endToken(): void {
    this.ended = true;
    this.#state = "data";
  }

  /** A tag ended: what it says of the head, its `<meta>`'s encoding, and the state it opens. */
  #emitTag(): void {
    this.#finishAttribute();
    this.#endToken();
    const name = this.#name;
    const allowed =
      ALLOWED_IN_HEAD.has(name) || (this.#start && (name === "html" || name === "head"));
    if (!allowed) {
      this.#leftHead = true;
    }
    if (!this.#start) {
      return;
    }
    this.#lastStart = name;
    if (name === "meta") {
      const encoding = metaEncoding(this.#attributes);
      if (encoding !== undefined) {
        this.found = encoding;
      }
    }
    this.#raw = RAW_TEXT.get(name) ?? "data";
    this.#state = this.#raw;
  }
}

/** The encoding a `<meta>` of `attributes` declares (Blink's `EncodingFromMetaAttributes`). */
function metaEncoding(attributes: readonly (readonly [string, string])[]): string | undefined {
  let pragma = false;
  let charset: readonly [string, boolean] | undefined;
  for (const [name, value] of attributes) {
    if (name === "http-equiv") {
      pragma ||= asciiLower(value) === "content-type";
    } else if (name === "charset") {
      charset = [value, true];
    } else if (name === "content" && charset?.[1] !== true) {
      const extracted = extractCharset(value);
      if (extracted !== "") {
        charset = [extracted, false];
      }
    }
  }
  if (charset === undefined) {
    return undefined;
  }
  const [label, fromCharset] = charset;
  if (!fromCharset && !pragma) {
    return undefined;
  }
  const encoding = forLabel(label.replace(HTML_SPACES, ""));
  if (encoding === undefined) {
    return undefined;
  }
  return encoding === X_USER_DEFINED ? WINDOWS_1252 : byteBased(encoding);
}

/**
 * The label a `<meta>`'s `content` names after `charset=`; empty for none:
 * the HTML standard's algorithm, as Blink's `ExtractCharset` runs it.
 */
function extractCharset(value: string): string {
  const lowered = asciiLower(value);
  const length = value.length;
  const spaceAt = (position: number): boolean => value.charCodeAt(position) <= 0x20;
  let start = 0;
  for (
    let at = lowered.indexOf("charset", start);
    at !== -1;
    at = lowered.indexOf("charset", start)
  ) {
    let position = at + "charset".length;
    while (position < length && spaceAt(position)) {
      position += 1;
    }
    if (position >= length || value[position] !== "=") {
      start = position;
      continue;
    }
    position += 1;
    while (position < length && spaceAt(position)) {
      position += 1;
    }
    const quote = value[position] === '"' || value[position] === "'" ? value[position] : undefined;
    if (quote !== undefined) {
      position += 1;
    }
    if (position >= length) {
      break;
    }
    let end = position;
    while (
      end < length &&
      (quote === undefined
        ? !spaceAt(end) && !"\"';".includes(value[end] ?? "")
        : value[end] !== quote)
    ) {
      end += 1;
    }
    if (quote !== undefined && end === length) {
      break;
    }
    return value.slice(position, end);
  }
  return "";
}
