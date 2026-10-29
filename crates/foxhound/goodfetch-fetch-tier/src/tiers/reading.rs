//! The browser tier's document reader (spec #579; ADR 0119): the
//! interception settings Foxhound sets on a handler's browser page, no
//! part of its value (ADR 0087), so the browser reads each document as the
//! document tiers read it and never starts a download.
//!
//! - A response whose type is no HTML or XHTML document, its type its
//!   `Content-Type`'s or, with none, the one Chromium's sniffer gives its
//!   body, is read whole at the interception and its navigation stopped:
//!   the page holds no document, whose calls fail with
//!   `response_not_document`, and `response.body` reads the bytes kept.
//! - The crawl request's own document is first judged by its Label's
//!   `crawl.content_types`, as the document tiers judge it: a type it does
//!   not take, of a response that would reach the handler, is stopped from
//!   its head alone, its body unread, and its request ends `skipped`.
//! - A document is read in ADR 0119's order: where `fetch.encoding` (the
//!   crawl request's own document only) or a valid UTF-8 body that
//!   declares nothing decides, the browser is given that charset; a BOM,
//!   the `Content-Type` charset and a `<meta>` the browser reads itself.
//!   The handler's response keeps the site's headers either way.
//!
//! A handler's own navigations are not judged by `crawl.content_types`.

use std::collections::BTreeSet;
use std::sync::Arc;

use bytes::Bytes;
use goodfetch_engine::runs::Run;
use goodfetch_page::page_value::{DocumentReader, ReadDocument, Reading};

use super::{Served, lock};
use crate::charset::{self, Kind, Read, Source};
use crate::content_types::{Accepts, declared, is_document, typed};
use crate::gate::HandOver;

/// What the document reader learned of one browser page's documents.
#[derive(Clone, Default)]
pub(super) struct BrowserRead {
    /// The type of its crawl request's own document, when its Label does
    /// not take it: its request ends `skipped`.
    pub(super) refused: Option<String>,
    /// The encoding its crawl request's own document was read in, and
    /// where that came from; none when only the browser's guess read it.
    pub(super) own: Option<Read>,
    /// While the page holds no document: what its response was.
    pub(super) not_document: Option<NotDocument>,
    /// The URL of the last main-frame document it reached, its own or a
    /// later one a script or the handler navigated to.
    pub(super) ended_at: Option<String>,
}

/// A response a browser page holds instead of a document.
#[derive(Clone)]
pub(super) struct NotDocument {
    /// Its type.
    pub(super) content_type: String,
    /// The browser's ID of its request, by which `response.body` asks.
    pub(super) request_id: String,
    /// Its body, after any content coding.
    pub(super) body: Bytes,
    /// The Run its body is stored in once `response.body` asks.
    pub(super) run: Run,
}

/// The document reader of the browser pages one attempt hands over.
pub(super) struct BrowserReader {
    pub(super) served: Arc<Served>,
    pub(super) hand_over: Arc<HandOver>,
    /// `crawl.handle_statuses`: the statuses besides the 2xx whose
    /// response reaches the handler, and so is judged by its type.
    pub(super) handled: BTreeSet<u16>,
}

impl std::fmt::Debug for BrowserReader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserReader")
            .field("run", self.hand_over.run.id())
            .field("handled", &self.handled)
            .finish_non_exhaustive()
    }
}

impl BrowserReader {
    /// Changes what the page `document` is on learned, as `change` says.
    fn learn(&self, document: &ReadDocument<'_>, change: impl FnOnce(&mut BrowserRead)) {
        let mut pages = lock(&self.served.pages);
        change(pages.browser.entry(document.page_id.clone()).or_default());
    }
}

impl DocumentReader for BrowserReader {
    fn read(&self, document: &ReadDocument<'_>, body: Option<&[u8]>) -> Reading {
        self.learn(document, |read| {
            read.ended_at = Some(document.url.to_owned());
        });
        let header = document
            .headers
            .iter()
            .find(|(name, _)| name == "content-type")
            .map(|(_, value)| value.as_str());
        let path = url::Url::parse(document.url)
            .map(|url| url.path().to_owned())
            .unwrap_or_default();
        let (essence, charset) = match (declared(header), body) {
            (Some(header), _) => typed(Some(header), &[], &path),
            (None, None) => return Reading::Body,
            (None, Some(body)) => typed(None, body, &path),
        };
        if document.own {
            let accepts = Accepts {
                types: self.hand_over.content_types.clone(),
                handled: self.handled.clone(),
            };
            if let Some(refused) = accepts.refused(document.status, &essence) {
                self.learn(document, |read| read.refused = Some(refused));
                return Reading::Stop;
            }
        }
        let Some(body) = body else {
            return Reading::Body;
        };
        if !is_document(&essence) {
            let not_document = NotDocument {
                content_type: essence,
                request_id: document.request_id.to_owned(),
                body: Bytes::copy_from_slice(body),
                run: self.hand_over.run.clone(),
            };
            self.learn(document, |read| read.not_document = Some(not_document));
            return Reading::Stop;
        }
        let forced = self.hand_over.encoding.filter(|_| document.own);
        let found = charset::read(body, Kind::of(&essence), charset.as_deref(), forced);
        self.learn(document, |read| {
            read.not_document = None;
            if document.own {
                read.own = found;
            }
        });
        match found {
            Some(Read {
                encoding,
                source: Source::Setting | Source::Utf8,
            }) => Reading::Charset(encoding.name().to_owned()),
            _ => Reading::AsItCame,
        }
    }
}
