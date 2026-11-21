//! The read of a crawl request's own document as its decoded bytes
//! arrive (issue 2188): its text, found and decoded as it streams, and the
//! untouched readers' pass over that text, on the fetch's own task.

use bytes::Bytes;
use goodfetch_encoding::{Kind, content_type};

use super::OwnDocument;
use crate::charset::{Stream, Streamed};
use crate::content_types::is_document;
use crate::untouched::{Passed, UntouchedPass, UntouchedReaders};

/// What reads a crawl request's own document as its decoded bytes arrive:
/// its text, and the readers' pass over the text handed out.
pub(super) struct OwnRead {
    stream: Stream,
    /// Boxed: the read is held across the body's awaits, and each of the
    /// futures above a fetch, some holding several, would carry the pass.
    pass: Option<Box<UntouchedPass>>,
}

impl OwnRead {
    /// The read of `own`, of the type its head `declared`: none when that
    /// type is no HTML or XHTML.
    pub(super) fn of((own, declared): (OwnDocument<'_>, &str)) -> Option<Self> {
        let (essence, charset) = content_type(declared);
        is_document(&essence).then(|| Self {
            stream: Stream::new(Kind::of(&essence), charset, own.encoding),
            pass: own.readers.and_then(UntouchedReaders::pass).map(Box::new),
        })
    }

    /// Reads `decoded`, the body decoded so far.
    pub(super) fn advance(&mut self, decoded: &[u8]) {
        let Self { stream, pass } = self;
        stream.advance(decoded, &mut |text| {
            if let Some(pass) = pass.as_mut() {
                pass.write(text);
            }
        });
    }

    /// The text of `body`, the whole body decoded, and what the pass read
    /// of it; neither when the text is still to find.
    pub(super) fn finish(self, body: &Bytes) -> (Option<Streamed>, Option<Passed>) {
        let Self { stream, mut pass } = self;
        let text = stream.finish(body, &mut |text| {
            if let Some(pass) = pass.as_mut() {
                pass.write(text);
            }
        });
        let passed = text.as_ref().and(pass).map(|pass| pass.end());
        (text, passed)
    }
}
