//! The encoding of a crawl page's document, found in one order on every
//! tier (ADR 0119; pitfalls H16, H17): the crawl request's own document
//! and each document the handler's own navigations load.
//!
//! 1. a BOM;
//! 2. `fetch.encoding`, for the crawl request's own document;
//! 3. the `Content-Type` charset, when WHATWG Encoding knows its label;
//! 4. for HTML, Chrome's `<meta>` prescan (Blink's `HTMLMetaCharsetParser`),
//!    else an XML declaration at the very start, as Blink's
//!    `TextResourceDecoder` reads one; for an XML type, such as
//!    `application/xhtml+xml`, its XML declaration alone;
//! 5. UTF-8, when the whole body is valid UTF-8, ASCII included.
//!
//! Any other body could only be guessed: [`read`] answers `None`, which a
//! non-browser tier escalates. Every label maps as WHATWG Encoding maps
//! it; a declaration in the document's own bytes reads a UTF-16 label as
//! UTF-8 and the `<meta>` prescan `x-user-defined` as windows-1252, as
//! Blink's decoder does. Pure: no I/O and no task, so a benchmark compiles
//! it in (ADR 0071).
//!
//! A [`Stream`] follows the same order as a document's bytes arrive
//! (issue 2187): it finds the encoding as soon as the bytes received so
//! far settle it, decodes a document in another encoding than UTF-8 as
//! its bytes arrive, and reads a UTF-8 document in place, its body kept
//! once.

use std::cell::Cell;

use bytes::Bytes;
use bytestring::ByteString;
use encoding_rs::{CoderResult, Decoder, Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};
use goodfetch_protocol::foxhound::provenance::EncodingSource;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};

/// Where a document's encoding came from, as Provenance names it (ADR
/// 0119).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// Its byte order mark.
    Bom,
    /// `fetch.encoding`.
    Setting,
    /// The `Content-Type` charset.
    Header,
    /// A declaration in its own bytes: a `<meta>` or an XML declaration.
    Meta,
    /// Nothing declared, and the body is valid UTF-8.
    Utf8,
}

impl Source {
    /// Its name, as Provenance records it.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Bom => "bom",
            Self::Setting => "setting",
            Self::Header => "header",
            Self::Meta => "meta",
            Self::Utf8 => "utf8",
        }
    }

    /// Its name in a Record's Provenance.
    pub(crate) fn provenance(self) -> EncodingSource {
        match self {
            Self::Bom => EncodingSource::Bom,
            Self::Setting => EncodingSource::Setting,
            Self::Header => EncodingSource::Header,
            Self::Meta => EncodingSource::Meta,
            Self::Utf8 => EncodingSource::Utf8,
        }
    }
}

/// The encoding a document is read in, and where it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Read {
    pub(crate) encoding: &'static Encoding,
    pub(crate) source: Source,
}

/// Which declaration in its own bytes a document's type reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// `text/html`: the `<meta>` prescan, then an XML declaration.
    Html,
    /// An XML type: its XML declaration.
    Xml,
    /// Any other type: none.
    Other,
}

impl Kind {
    /// The kind of the MIME type whose essence is `essence`, lowercase.
    pub(crate) fn of(essence: &str) -> Self {
        match essence {
            "text/html" => Self::Html,
            "application/xhtml+xml" | "text/xml" | "application/xml" => Self::Xml,
            other if other.ends_with("+xml") => Self::Xml,
            _ => Self::Other,
        }
    }
}

/// The essence of the `Content-Type` value `value`, lowercase, and its
/// `charset` parameter, unquoted.
pub(crate) fn content_type(value: &str) -> (String, Option<&str>) {
    let mut parts = value.split(';');
    let essence = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
    let charset = parts.find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches('"'))
    });
    (essence, charset)
}

/// The encoding `body`, a document of `kind` whose `Content-Type` charset
/// is `charset`, is read in, `forced` being `fetch.encoding`'s; `None`
/// when only a guess could read it.
pub(crate) fn read(
    body: &[u8],
    kind: Kind,
    charset: Option<&str>,
    forced: Option<&'static Encoding>,
) -> Option<Read> {
    read_under(body, kind, (labelled(charset), forced))
}

/// The encoding the `Content-Type` charset `charset` names, when WHATWG
/// Encoding knows its label.
fn labelled(charset: Option<&str>) -> Option<&'static Encoding> {
    charset.and_then(|label| Encoding::for_label(label.as_bytes()))
}

/// [`read`], the `Content-Type` charset's encoding `header` already
/// found.
fn read_under(
    body: &[u8],
    kind: Kind,
    (header, forced): (Option<&'static Encoding>, Option<&'static Encoding>),
) -> Option<Read> {
    let found = |encoding, source| Some(Read { encoding, source });
    if let Some((encoding, _)) = Encoding::for_bom(body) {
        return found(encoding, Source::Bom);
    }
    if let Some(encoding) = forced {
        return found(encoding, Source::Setting);
    }
    if let Some(encoding) = header {
        return found(encoding, Source::Header);
    }
    let declared = match kind {
        Kind::Html => prescan(body).or_else(|| xml_declaration(body)),
        Kind::Xml => xml_declaration(body),
        Kind::Other => None,
    };
    if let Some(encoding) = declared {
        return found(encoding, Source::Meta);
    }
    std::str::from_utf8(body).is_ok().then_some(Read {
        encoding: UTF_8,
        source: Source::Utf8,
    })
}

/// `body` decoded in `read`'s encoding, its BOM left out, a byte that does
/// not decode replaced by U+FFFD.
pub(crate) fn decode(body: &[u8], read: Read) -> String {
    let (text, _) = if read.source == Source::Bom {
        read.encoding.decode_with_bom_removal(body)
    } else {
        read.encoding.decode_without_bom_handling(body)
    };
    text.into_owned()
}

/// How many bytes the prescan always reads, as Blink's
/// `kBytesToCheckUnconditionally`.
const UNCONDITIONAL: usize = 1024;

/// The encoding a `<meta>` of `bytes` declares, read as Chrome's prescan
/// reads it (Blink's `HTMLMetaCharsetParser`): the bytes tokenized as the
/// HTML standard's tokenizer does, each a Latin-1 character, the text of
/// a `script`, `style`, `title` and the like read as text; the first
/// `<meta>` whose `charset`, or whose `content` beside an
/// `http-equiv="content-type"`, names a known label wins; the scan stops
/// once a tag that is not allowed in the head appeared and 1,024 bytes
/// were read, checked as each token ends.
pub(crate) fn prescan(bytes: &[u8]) -> Option<&'static Encoding> {
    match prescanned(bytes) {
        Prescanned::Found(encoding) => Some(encoding),
        Prescanned::Ended | Prescanned::Unended => None,
    }
}

/// Where the prescan of a document's first bytes stands once they are
/// read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prescanned {
    /// A `<meta>` declared this encoding.
    Found(&'static Encoding),
    /// The scan stopped with no `<meta>` declaring one.
    Ended,
    /// The bytes ended before the scan did: the bytes after them decide.
    Unended,
}

/// [`prescan`] of `bytes`, the first bytes of a document or all of them:
/// the tokenizer reads one byte at a time and the scan decides on each
/// byte as it is read, so whatever it decided on these bytes it decides
/// on any document they begin.
fn prescanned(bytes: &[u8]) -> Prescanned {
    let tokenizer = Tokenizer::new(Prescan::default(), TokenizerOpts::default());
    let queue = BufferQueue::default();
    let mut character = [0; 4];
    for (consumed, byte) in bytes.iter().enumerate() {
        let sink = &tokenizer.sink;
        // Blink's tokenizer ends a run of text when a tag opens.
        if *byte == b'<' && sink.text.take() && sink.done(consumed) {
            return Prescanned::Ended;
        }
        let character = char::from(*byte).encode_utf8(&mut character);
        queue.push_back((&*character).into());
        let _ = tokenizer.feed(&queue);
        if let Some(found) = sink.found.get() {
            return Prescanned::Found(found);
        }
        if sink.ended.take() && sink.done(consumed + 1) {
            return Prescanned::Ended;
        }
    }
    Prescanned::Unended
}

/// What the prescan's tokenizer has seen so far.
#[derive(Default)]
struct Prescan {
    /// The encoding a `<meta>` declared.
    found: Cell<Option<&'static Encoding>>,
    /// Whether a tag not allowed in the head appeared.
    left_head: Cell<bool>,
    /// Whether a token other than text ended in the last byte fed.
    ended: Cell<bool>,
    /// Whether a run of text is open.
    text: Cell<bool>,
}

impl Prescan {
    /// Whether the scan stops, `consumed` bytes read.
    fn done(&self, consumed: usize) -> bool {
        self.left_head.get() && consumed >= UNCONDITIONAL
    }

    fn tag(&self, tag: &Tag) -> TokenSinkResult<()> {
        let name = &*tag.name;
        let start = tag.kind == TagKind::StartTag;
        let allowed = matches!(
            name,
            "script" | "noscript" | "style" | "link" | "meta" | "object" | "title" | "base"
        ) || (start && matches!(name, "html" | "head"));
        if !allowed {
            self.left_head.set(true);
        }
        if !start {
            return TokenSinkResult::Continue;
        }
        if name == "meta"
            && let Some(encoding) = meta_encoding(tag)
        {
            self.found.set(Some(encoding));
        }
        // Blink's `UpdateStateFor`, its parser's scripting flag off.
        match name {
            "title" | "textarea" => TokenSinkResult::RawData(RawKind::Rcdata),
            "plaintext" => TokenSinkResult::Plaintext,
            "script" => TokenSinkResult::RawData(RawKind::ScriptData),
            "style" | "iframe" | "xmp" | "noembed" | "noframes" => {
                TokenSinkResult::RawData(RawKind::Rawtext)
            }
            _ => TokenSinkResult::Continue,
        }
    }
}

impl TokenSink for Prescan {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<()> {
        match token {
            Token::TagToken(tag) => {
                self.ended.set(true);
                self.tag(&tag)
            }
            Token::CharacterTokens(_) | Token::NullCharacterToken => {
                self.text.set(true);
                TokenSinkResult::Continue
            }
            Token::CommentToken(_) | Token::DoctypeToken(_) => {
                self.ended.set(true);
                TokenSinkResult::Continue
            }
            Token::EOFToken | Token::ParseError(_) => TokenSinkResult::Continue,
        }
    }
}

/// The encoding the `<meta>` `tag` declares (Blink's
/// `EncodingFromMetaAttributes`), as Blink's decoder takes it from a
/// `<meta>`.
fn meta_encoding(tag: &Tag) -> Option<&'static Encoding> {
    let mut pragma = false;
    let mut charset: Option<(String, bool)> = None;
    for attribute in &tag.attrs {
        let value = &*attribute.value;
        match &*attribute.name.local {
            "http-equiv" => pragma |= value.eq_ignore_ascii_case("content-type"),
            "charset" => charset = Some((value.to_owned(), true)),
            "content" if !charset.as_ref().is_some_and(|(_, set)| *set) => {
                let extracted = extract_charset(value);
                if !extracted.is_empty() {
                    charset = Some((extracted.to_owned(), false));
                }
            }
            _ => {}
        }
    }
    let (label, from_charset) = charset?;
    if !from_charset && !pragma {
        return None;
    }
    let encoding = Encoding::for_label(label.trim_matches(html_space).as_bytes())?;
    Some(if encoding.name().eq_ignore_ascii_case("x-user-defined") {
        WINDOWS_1252
    } else {
        byte_based(encoding)
    })
}

/// Whether `character` is an HTML space.
fn html_space(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\n' | '\x0C' | '\r')
}

/// The label a `<meta>`'s `content` names after `charset=` (the HTML
/// standard's algorithm, as Blink's `ExtractCharset` runs it); empty for
/// none.
fn extract_charset(value: &str) -> &str {
    let bytes = value.as_bytes();
    let lowered = value.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lowered[from..].find("charset") {
        let mut pos = from + at + "charset".len();
        while pos < bytes.len() && bytes[pos] <= b' ' {
            pos += 1;
        }
        if bytes.get(pos) != Some(&b'=') {
            from = pos;
            continue;
        }
        pos += 1;
        while pos < bytes.len() && bytes[pos] <= b' ' {
            pos += 1;
        }
        let quote = bytes
            .get(pos)
            .copied()
            .filter(|byte| matches!(byte, b'"' | b'\''));
        if quote.is_some() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let mut end = pos;
        while end < bytes.len()
            && match quote {
                Some(quote) => bytes[end] != quote,
                None => bytes[end] > b' ' && !matches!(bytes[end], b'"' | b'\'' | b';'),
            }
        {
            end += 1;
        }
        if quote.is_some() && end == bytes.len() {
            break;
        }
        return &value[pos..end];
    }
    ""
}

/// The encoding an XML declaration at the very start of `bytes` names, as
/// Blink's `TextResourceDecoder` reads it: within `<?xml … >`, the first
/// `encoding`, then `=` and a quoted label, spaces allowed between.
pub(crate) fn xml_declaration(bytes: &[u8]) -> Option<&'static Encoding> {
    if bytes.len() < 8 || !bytes.starts_with(b"<?xml") {
        return None;
    }
    let declaration = &bytes[..bytes.iter().position(|byte| *byte == b'>')?];
    let at = declaration
        .windows(b"encoding".len())
        .position(|window| window == b"encoding")?;
    // Blink skips every byte up to the space as a space.
    let skip = |rest: &[u8]| -> usize { rest.iter().take_while(|byte| **byte <= b' ').count() };
    let mut rest = &declaration[at + b"encoding".len()..];
    rest = &rest[skip(rest)..];
    rest = rest.strip_prefix(b"=")?;
    rest = &rest[skip(rest)..];
    let (&quote, rest) = rest.split_first()?;
    if !matches!(quote, b'"' | b'\'') {
        return None;
    }
    let label = &rest[..rest.iter().position(|byte| *byte == quote)?];
    Encoding::for_label(label).map(byte_based)
}

/// What [`xml_declaration`] reads on any document `bytes` begin: pending
/// while the bytes after them could still change it, until its first
/// eight bytes, and the `>` closing a declaration that opened, arrived.
fn xml_declared(bytes: &[u8]) -> Found {
    const OPENING: &[u8] = b"<?xml";
    let opening = &bytes[..bytes.len().min(OPENING.len())];
    if !OPENING.starts_with(opening) {
        return Found::Undeclared;
    }
    if bytes.len() < 8 || !bytes.contains(&b'>') {
        return Found::Pending;
    }
    xml_declaration(bytes).map_or(Found::Undeclared, |encoding| {
        Found::Read(Read {
            encoding,
            source: Source::Meta,
        })
    })
}

/// `encoding`, or UTF-8 for UTF-16, as Blink's
/// `ClosestByteBasedEquivalent` maps a label a document declares itself.
fn byte_based(encoding: &'static Encoding) -> &'static Encoding {
    if encoding == UTF_16LE || encoding == UTF_16BE {
        UTF_8
    } else {
        encoding
    }
}

/// A document's text: the encoding it was read in, or none when only a
/// guess could read it, and its characters, its BOM left out and a byte
/// that does not decode replaced by U+FFFD; a guess reads it as UTF-8.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Text {
    pub(crate) read: Option<Read>,
    pub(crate) html: ByteString,
}

impl Text {
    /// `body`, a whole document of `kind` whose `Content-Type` charset is
    /// `charset`, read as [`read`] and [`decode`] read it, `forced` being
    /// `fetch.encoding`'s: a UTF-8 body in place.
    pub(crate) fn of(
        body: &Bytes,
        kind: Kind,
        charset: Option<&str>,
        forced: Option<&'static Encoding>,
    ) -> Self {
        Self::whole(body, read(body, kind, charset, forced))
    }

    /// `body`, read in `read`'s encoding, or as a guess.
    fn whole(body: &Bytes, read: Option<Read>) -> Self {
        let html = match read {
            Some(read) if read.encoding == UTF_8 => {
                let start = if read.source == Source::Bom { 3 } else { 0 };
                in_place(body.slice(start..))
            }
            Some(read) => ByteString::from(decode(body, read)),
            None => ByteString::from(String::from_utf8_lossy(body).into_owned()),
        };
        Self { read, html }
    }
}

/// UTF-8 `bytes` as text, in place when they are valid, else decoded
/// with each byte that does not decode replaced by U+FFFD.
fn in_place(bytes: Bytes) -> ByteString {
    ByteString::from_utf8(bytes).unwrap_or_else(|invalid| {
        let bytes = invalid.into_bytes();
        let (text, _) = UTF_8.decode_without_bom_handling(&bytes);
        ByteString::from(text.into_owned())
    })
}

/// The most bytes of a document a [`Stream`] prescans as they arrive:
/// the prescan runs again from the start each time the bytes received
/// double from 1,024, so its work stays under twice this. A document
/// whose prescan this does not end is read whole once it ended, on the
/// CPU pool.
const PRESCANNED_AS_THEY_ARRIVE: usize = 64 * 1024;

/// A document's text read in ADR 0119's order as its decoded bytes
/// arrive: the encoding found as soon as the bytes received settle it, as
/// [`read`] finds it on the whole body; the text of another encoding than
/// UTF-8 decoded as the bytes arrive, as the codings are; a UTF-8 body
/// left to read in place on the CPU pool ([`Streamed::text`]), which
/// checks it once. A body that broke off or passed a limit is never
/// finished, so it leaves nothing read.
pub(crate) struct Stream {
    kind: Kind,
    header: Option<&'static Encoding>,
    forced: Option<&'static Encoding>,
    phase: Phase,
}

/// How far a [`Stream`] read.
enum Phase {
    /// No encoding found yet: the prescan runs again once `prescan_at`
    /// bytes arrived, none past [`PRESCANNED_AS_THEY_ARRIVE`], and
    /// `prescanned` once it ended with no `<meta>`.
    Finding { prescan_at: usize, prescanned: bool },
    /// Found as UTF-8: the body is read in place once whole.
    InPlace(Read),
    /// Nothing declared: UTF-8 when the whole body is valid, else a guess.
    Undeclared,
    /// Found as another encoding: its characters, decoded as the bytes
    /// arrive, `fed` of them so far.
    Decoding {
        read: Read,
        decoder: Decoder,
        fed: usize,
        text: String,
    },
}

/// What the bytes received so far say of a document's encoding.
enum Found {
    /// Its encoding.
    Read(Read),
    /// Nothing declared.
    Undeclared,
    /// Not yet settled.
    Pending,
}

impl Stream {
    /// The stream of a document of `kind` whose `Content-Type` charset is
    /// `charset`, `forced` being `fetch.encoding`'s.
    pub(crate) fn new(
        kind: Kind,
        charset: Option<&str>,
        forced: Option<&'static Encoding>,
    ) -> Self {
        Self {
            kind,
            header: labelled(charset),
            forced,
            phase: Phase::Finding {
                prescan_at: UNCONDITIONAL,
                prescanned: false,
            },
        }
    }

    /// Reads `decoded`, every decoded byte of the body received so far,
    /// the bytes of an earlier call first.
    pub(crate) fn advance(&mut self, decoded: &[u8]) {
        if let Phase::Finding {
            prescan_at,
            prescanned,
        } = &mut self.phase
        {
            match found(
                decoded,
                self.kind,
                (self.header, self.forced),
                (prescan_at, prescanned),
            ) {
                Found::Pending => return,
                Found::Undeclared => self.phase = Phase::Undeclared,
                Found::Read(read) if read.encoding == UTF_8 => self.phase = Phase::InPlace(read),
                Found::Read(read) => {
                    self.phase = Phase::Decoding {
                        read,
                        decoder: if read.source == Source::Bom {
                            read.encoding.new_decoder_with_bom_removal()
                        } else {
                            read.encoding.new_decoder_without_bom_handling()
                        },
                        fed: 0,
                        text: String::new(),
                    };
                }
            }
        }
        if let Phase::Decoding {
            decoder, fed, text, ..
        } = &mut self.phase
        {
            feed(decoder, &decoded[*fed..], text, false);
            *fed = decoded.len();
        }
    }

    /// What it read of `body`, the whole body, once that ended: the
    /// encoding of a body of [`PRESCANNED_AS_THEY_ARRIVE`] bytes at most
    /// found on it whole when its bytes did not settle it; none for a
    /// longer one, which [`Text::of`] then reads on the CPU pool.
    pub(crate) fn finish(mut self, body: &Bytes) -> Option<Streamed> {
        self.advance(body);
        let ended = match self.phase {
            Phase::Finding { .. } if body.len() > PRESCANNED_AS_THEY_ARRIVE => return None,
            Phase::Finding { .. } => {
                match read_under(body, self.kind, (self.header, self.forced)) {
                    Some(read) if read.encoding == UTF_8 => Ended::InPlace(read),
                    Some(read) => Ended::Decoded(Text {
                        read: Some(read),
                        html: ByteString::from(decode(body, read)),
                    }),
                    None => Ended::Undeclared,
                }
            }
            Phase::InPlace(read) => Ended::InPlace(read),
            Phase::Undeclared => Ended::Undeclared,
            Phase::Decoding {
                read,
                mut decoder,
                mut text,
                ..
            } => {
                feed(&mut decoder, &[], &mut text, true);
                text.shrink_to_fit();
                Ended::Decoded(Text {
                    read: Some(read),
                    html: ByteString::from(text),
                })
            }
        };
        Some(Streamed {
            forced: self.forced,
            ended,
        })
    }
}

/// What a [`Stream`] read of a whole document, under the
/// `fetch.encoding` `forced`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Streamed {
    forced: Option<&'static Encoding>,
    ended: Ended,
}

/// How a [`Stream`] ended.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ended {
    /// Found as UTF-8.
    InPlace(Read),
    /// Nothing declared.
    Undeclared,
    /// Decoded as its bytes arrived.
    Decoded(Text),
}

impl Streamed {
    /// Whether it was read under `forced`, `fetch.encoding`'s: a response
    /// a coalesced fetch shared was read under the leading request's.
    pub(crate) fn read_under(&self, forced: Option<&'static Encoding>) -> bool {
        self.forced == forced
    }

    /// The text of `body`, the whole body it read, as [`Text::of`] reads
    /// it: a UTF-8 body in place, checked once.
    pub(crate) fn text(&self, body: &Bytes) -> Text {
        match &self.ended {
            Ended::InPlace(read) => Text::whole(body, Some(*read)),
            Ended::Undeclared => match ByteString::try_from(body.clone()) {
                Ok(html) => Text {
                    read: Some(Read {
                        encoding: UTF_8,
                        source: Source::Utf8,
                    }),
                    html,
                },
                Err(_) => Text::whole(body, None),
            },
            Ended::Decoded(text) => text.clone(),
        }
    }
}

/// What `bytes`, the first bytes of a document of `kind`, say of its
/// encoding in [`read`]'s order, `header` being the `Content-Type`
/// charset's and `forced` `fetch.encoding`'s; the prescan run only once
/// `prescan_at` bytes arrived, and `ended` once it ended with no
/// `<meta>`.
fn found(
    bytes: &[u8],
    kind: Kind,
    (header, forced): (Option<&'static Encoding>, Option<&'static Encoding>),
    (prescan_at, ended): (&mut usize, &mut bool),
) -> Found {
    const BOMS: [&[u8]; 3] = [b"\xEF\xBB\xBF", b"\xFE\xFF", b"\xFF\xFE"];
    let read = |encoding, source| Found::Read(Read { encoding, source });
    if let Some((encoding, _)) = Encoding::for_bom(bytes) {
        return read(encoding, Source::Bom);
    }
    if bytes.len() < 3 && BOMS.iter().any(|bom| bom.starts_with(bytes)) {
        return Found::Pending;
    }
    if let Some(encoding) = forced {
        return read(encoding, Source::Setting);
    }
    if let Some(encoding) = header {
        return read(encoding, Source::Header);
    }
    if kind == Kind::Html && !*ended {
        if bytes.len() < *prescan_at || *prescan_at > PRESCANNED_AS_THEY_ARRIVE {
            return Found::Pending;
        }
        let scanned = &bytes[..bytes.len().min(PRESCANNED_AS_THEY_ARRIVE)];
        match prescanned(scanned) {
            Prescanned::Found(encoding) => return read(encoding, Source::Meta),
            Prescanned::Ended => *ended = true,
            Prescanned::Unended => {
                *prescan_at = scanned.len().saturating_mul(2);
                return Found::Pending;
            }
        }
    }
    if kind == Kind::Other {
        return Found::Undeclared;
    }
    xml_declared(bytes)
}

/// Decodes `bytes` with `decoder` onto `text`, `last` once the body
/// ended.
fn feed(decoder: &mut Decoder, mut bytes: &[u8], text: &mut String, last: bool) {
    loop {
        text.reserve(
            decoder
                .max_utf8_buffer_length(bytes.len())
                .unwrap_or(bytes.len()),
        );
        let (result, read, _) = decoder.decode_to_string(bytes, text, last);
        bytes = &bytes[read..];
        if result == CoderResult::InputEmpty {
            return;
        }
    }
}

/// The `<meta>` prescan and the decoding order over arbitrary bytes, as a
/// crawl's documents hold them: the fuzz target's body (ADR 0067), the
/// stream fed in two parts reading as the whole body does.
pub fn fuzz_prescan(data: &[u8]) {
    let _ = prescan(data);
    let _ = xml_declaration(data);
    let body = Bytes::copy_from_slice(data);
    for kind in [Kind::Html, Kind::Xml, Kind::Other] {
        if let Some(read) = read(data, kind, None, None) {
            let _ = decode(data, read);
        }
        let mut stream = Stream::new(kind, None, None);
        stream.advance(&data[..data.len() / 2]);
        stream.advance(data);
        if let Some(streamed) = stream.finish(&body) {
            assert_eq!(
                streamed.text(&body),
                Text::of(&body, kind, None, None),
                "a document read as it arrives reads as the whole body does"
            );
        }
    }
}
