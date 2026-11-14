//! The formats a code-free crawl reads of a document a fetch tier served,
//! read in the document's one hand-off to the analysis pool, on a copy of
//! its tree, before its Page signature takes the tree as served (issue
//! 2189): `page.formats` then answers from them, with no read of its own.
//! The types behind [`ServedFormatsAhead`] and [`FormatsReadAhead`], the
//! seams `page_value` names.

use std::sync::Arc;

use bytestring::ByteString;
use goodfetch_counted_work::{CountedWork, ParsedHtml};
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::RunSettings;
use goodfetch_extractor::{BuiltIn, Chosen, Serialization};
use goodfetch_protocol::page::{FormatsRequest, PageFormats, PageId};
use serde_json::Value;

use super::clean_up::{self, FormatsRead, ShareMeasure};
use crate::driver::SerializedDocument;
use crate::extractor;
use crate::interception::Cosmetic;
use crate::page_value::{FormatsReadAhead, ServedFormatsAhead};

/// How the formats of a Run's served documents are read ahead, as
/// `page.formats` reads them: the request, the main-content step, the
/// share's measure and the cosmetic rules, each made once for the Run.
pub(crate) struct ServedFormatsReader {
    request: FormatsRequest,
    extractor: Chosen,
    measure: ShareMeasure,
    cosmetic: Option<Cosmetic>,
}

/// The formats a [`ServedFormatsReader`] read of one served document, and
/// what its rules did, which `page.formats` answers a call asking for the
/// same request with.
pub(crate) struct ServedFormats {
    request: FormatsRequest,
    formats: PageFormats,
    read: Option<FormatsRead>,
}

impl ServedFormatsReader {
    /// The reader of the formats `request` names of `run`'s served
    /// documents, read with `label`, their Label's `content.extractor`, and
    /// `cosmetic`, the Run's cosmetic rules for them.
    pub(crate) fn of(
        run: &Run,
        request: FormatsRequest,
        label: Option<BuiltIn>,
        cosmetic: Option<Cosmetic>,
    ) -> Self {
        let resolved = run.settings().map(RunSettings::resolved);
        Self {
            request,
            extractor: extractor::served(resolved, label),
            measure: ShareMeasure::of(resolved),
            cosmetic,
        }
    }
}

impl ServedFormatsAhead for ServedFormatsReader {
    fn read(
        &self,
        (html, url, base_url): (&ByteString, &str, &str),
        kept: Option<&ParsedHtml>,
        work: &CountedWork,
    ) -> Arc<dyn FormatsReadAhead> {
        let document = SerializedDocument {
            html: html.clone(),
            url: url.to_owned(),
            base_url: base_url.to_owned(),
        };
        let (formats, read) = clean_up::read(
            (&document, kept),
            Serialization::Served,
            self.request,
            (self.extractor.extractor(), self.measure),
            (self.cosmetic.as_ref(), &[]),
            work,
        );
        Arc::new(ServedFormats {
            request: self.request,
            formats,
            read,
        })
    }
}

impl FormatsReadAhead for ServedFormats {
    fn answers(&self, request: FormatsRequest) -> Option<&PageFormats> {
        (self.request == request).then_some(&self.formats)
    }

    fn entry(&self, page_id: &PageId) -> Option<Value> {
        self.read.as_ref().map(|read| read.entry(page_id, None))
    }
}
