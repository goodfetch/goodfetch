//! The encoding of a crawl page's document, found in one order on every
//! tier (ADR 0119; pitfalls H16, H17): the crawl request's own document
//! and each document the handler's own navigations load.
//!
//! 1. a BOM;
//! 2. `fetch.encoding`, for the crawl request's own document;
//! 3. the `Content-Type` charset, when WHATWG Encoding knows its label;
//! 4. the declaration in its own bytes, as `goodfetch_encoding` reads it:
//!    for HTML, Chrome's `<meta>` prescan (Blink's `HTMLMetaCharsetParser`),
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

use bytes::Bytes;
use bytestring::ByteString;
use encoding_rs::{CoderResult, Decoder, Encoding, UTF_8};
use goodfetch_encoding::{Kind, Prescanned, UNCONDITIONAL, declared, prescanned, xml_declaration};
use goodfetch_protocol::foxhound::provenance::EncodingSource;

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
    if let Some(encoding) = declared(body, kind) {
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
    in_place_checked(bytes).0
}

/// [`in_place`], and whether `bytes` were valid, so read in place.
fn in_place_checked(bytes: Bytes) -> (ByteString, bool) {
    match ByteString::from_utf8(bytes) {
        Ok(text) => (text, true),
        Err(invalid) => {
            let bytes = invalid.into_bytes();
            let (text, _) = UTF_8.decode_without_bom_handling(&bytes);
            (ByteString::from(text.into_owned()), false)
        }
    }
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
///
/// It hands out the document's characters as they are read, for a reader
/// of the text as it arrives (issue 2188): once the encoding is found,
/// the UTF-8 bytes received, after a BOM, or the characters decoded.
/// [`Streamed::text`] tells whether the text it ends with is what it
/// handed out: always, but for a UTF-8 or undeclared body holding bytes
/// that do not decode.
pub(crate) struct Stream {
    kind: Kind,
    header: Option<&'static Encoding>,
    forced: Option<&'static Encoding>,
    phase: Phase,
    /// Where the text not yet handed out starts: in the decoded bytes
    /// for a UTF-8 or undeclared body, in the decoded characters for
    /// another encoding.
    handed: usize,
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
            handed: 0,
        }
    }

    /// Reads `decoded`, every decoded byte of the body received so far,
    /// the bytes of an earlier call first, handing `to` the characters
    /// read that it was not handed yet.
    pub(crate) fn advance(&mut self, decoded: &[u8], to: &mut impl FnMut(&[u8])) {
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
                Found::Read(read) if read.encoding == UTF_8 => {
                    self.handed = if read.source == Source::Bom { 3 } else { 0 };
                    self.phase = Phase::InPlace(read);
                }
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
        match &mut self.phase {
            Phase::Finding { .. } => {}
            Phase::InPlace(_) | Phase::Undeclared => {
                hand(decoded, &mut self.handed, to);
            }
            Phase::Decoding {
                decoder, fed, text, ..
            } => {
                feed(decoder, &decoded[*fed..], text, false);
                *fed = decoded.len();
                hand(text.as_bytes(), &mut self.handed, to);
            }
        }
    }

    /// What it read of `body`, the whole body, once that ended: the
    /// encoding of a body of [`PRESCANNED_AS_THEY_ARRIVE`] bytes at most
    /// found on it whole when its bytes did not settle it; none for a
    /// longer one, which [`Text::of`] then reads on the CPU pool. `to` is
    /// handed the characters it was not handed yet, those of a body whose
    /// encoding is found on it whole included.
    pub(crate) fn finish(mut self, body: &Bytes, to: &mut impl FnMut(&[u8])) -> Option<Streamed> {
        self.advance(body, to);
        let ended = match self.phase {
            Phase::Finding { .. } if body.len() > PRESCANNED_AS_THEY_ARRIVE => return None,
            Phase::Finding { .. } => {
                match read_under(body, self.kind, (self.header, self.forced)) {
                    Some(read) if read.encoding == UTF_8 => {
                        let start = if read.source == Source::Bom { 3 } else { 0 };
                        to(body.get(start..).unwrap_or_default());
                        Ended::InPlace(read)
                    }
                    Some(read) => {
                        let text = decode(body, read);
                        to(text.as_bytes());
                        Ended::Decoded(Text {
                            read: Some(read),
                            html: ByteString::from(text),
                        })
                    }
                    None => {
                        to(&body[..]);
                        Ended::Undeclared
                    }
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
                hand(text.as_bytes(), &mut self.handed, to);
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
    /// it: a UTF-8 body in place, checked once; and whether its
    /// characters are those the stream handed out, which they are but
    /// for a UTF-8 or undeclared body holding bytes that do not decode.
    pub(crate) fn text(&self, body: &Bytes) -> (Text, bool) {
        match &self.ended {
            Ended::InPlace(read) => {
                let start = if read.source == Source::Bom { 3 } else { 0 };
                let (html, valid) = in_place_checked(body.slice(start.min(body.len())..));
                (
                    Text {
                        read: Some(*read),
                        html,
                    },
                    valid,
                )
            }
            Ended::Undeclared => match ByteString::try_from(body.clone()) {
                Ok(html) => (
                    Text {
                        read: Some(Read {
                            encoding: UTF_8,
                            source: Source::Utf8,
                        }),
                        html,
                    },
                    true,
                ),
                Err(_) => (Text::whole(body, None), false),
            },
            Ended::Decoded(text) => (text.clone(), true),
        }
    }
}

/// Hands `to` the bytes of `read` from `handed` on, then marks them handed.
fn hand(read: &[u8], handed: &mut usize, to: &mut impl FnMut(&[u8])) {
    if let Some(rest) = read.get(*handed..).filter(|rest| !rest.is_empty()) {
        to(rest);
        *handed = read.len();
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
    let _ = goodfetch_encoding::prescan(data);
    let _ = xml_declaration(data);
    let body = Bytes::copy_from_slice(data);
    for kind in [Kind::Html, Kind::Xml, Kind::Other] {
        if let Some(read) = read(data, kind, None, None) {
            let _ = decode(data, read);
        }
        let mut stream = Stream::new(kind, None, None);
        let mut handed = Vec::new();
        let mut to = |text: &[u8]| handed.extend_from_slice(text);
        stream.advance(&data[..data.len() / 2], &mut to);
        stream.advance(data, &mut to);
        if let Some(streamed) = stream.finish(&body, &mut to) {
            let (text, as_handed) = streamed.text(&body);
            assert_eq!(
                text,
                Text::of(&body, kind, None, None),
                "a document read as it arrives reads as the whole body does"
            );
            assert_eq!(
                as_handed,
                handed == text.html.as_bytes()[..],
                "a stream tells whether it handed out its text"
            );
        }
    }
}
