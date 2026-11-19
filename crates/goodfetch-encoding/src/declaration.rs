//! The Encoding a document declares in its own bytes, step 4 of ADR
//! 0119's order: for HTML, Chrome's `<meta>` prescan (Blink's
//! `HTMLMetaCharsetParser`), else an XML declaration at the very start, as
//! Blink's `TextResourceDecoder` reads one; for an XML type, such as
//! `application/xhtml+xml`, its XML declaration alone. A declaration reads
//! a UTF-16 label as UTF-8 and the `<meta>` prescan `x-user-defined` as
//! windows-1252, as Blink's decoder does.

use std::cell::Cell;

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};

use crate::media_type::Kind;

/// The Encoding `body`, a document of `kind`, declares in its own bytes:
/// for HTML, a `<meta>` [`prescan`] finds, else its
/// [`xml_declaration`]; for an XML type, its XML declaration; none for any
/// other type.
#[must_use]
pub fn declared(body: &[u8], kind: Kind) -> Option<&'static Encoding> {
    match kind {
        Kind::Html => prescan(body).or_else(|| xml_declaration(body)),
        Kind::Xml => xml_declaration(body),
        Kind::Other => None,
    }
}

/// How many bytes the prescan always reads, as Blink's
/// `kBytesToCheckUnconditionally`.
pub const UNCONDITIONAL: usize = 1024;

/// The encoding a `<meta>` of `bytes` declares, read as Chrome's prescan
/// reads it (Blink's `HTMLMetaCharsetParser`): the bytes tokenized as the
/// HTML standard's tokenizer does, each a Latin-1 character, the text of
/// a `script`, `style`, `title` and the like read as text; the first
/// `<meta>` whose `charset`, or whose `content` beside an
/// `http-equiv="content-type"`, names a known label wins; the scan stops
/// once a tag that is not allowed in the head appeared and 1,024 bytes
/// were read, checked as each token ends.
#[must_use]
pub fn prescan(bytes: &[u8]) -> Option<&'static Encoding> {
    match prescanned(bytes) {
        Prescanned::Found(encoding) => Some(encoding),
        Prescanned::Ended | Prescanned::Unended => None,
    }
}

/// Where the prescan of a document's first bytes stands once they are
/// read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prescanned {
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
#[must_use]
pub fn prescanned(bytes: &[u8]) -> Prescanned {
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
#[must_use]
pub fn xml_declaration(bytes: &[u8]) -> Option<&'static Encoding> {
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

/// `encoding`, or UTF-8 for UTF-16, as Blink's
/// `ClosestByteBasedEquivalent` maps a label a document declares itself.
fn byte_based(encoding: &'static Encoding) -> &'static Encoding {
    if encoding == UTF_16LE || encoding == UTF_16BE {
        UTF_8
    } else {
        encoding
    }
}

#[cfg(test)]
mod tests;
