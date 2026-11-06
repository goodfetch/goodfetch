//! A page's signature (spec #581; ADR 0005): the SHA-256 of its document's
//! title and the text of its main content, each with its white space
//! collapsed, by which the Frontier tells whether two URLs serve one page.
//! Links, navigation and markup outside the main content's text take no
//! part, so two URLs of one page whose links carry their own parameters
//! still match. The main content is the Run's chosen Extractor's, with the
//! fallback the formats read it with (ADR 0138), one run of the step per
//! page.

use goodfetch_counted_work::{CountedWork, ParsedHtml, Row};
use sha2::{Digest as _, Sha256};

use crate::extractor::Extractor;
use crate::formats::Document;
use crate::main_content::title_and_main_text_of;

/// The page signature of `document`, its main content found by
/// `extractor`: the SHA-256 of its title, a NUL and the text of its main
/// content, which neither holds, so text moved from one to the other makes
/// another signature. A document with no main content is signed by its
/// title alone.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0138: the Page signature of a served page with no untouched tree of its own, parsed for it alone"
)]
pub fn page_signature(
    document: &Document<'_>,
    extractor: &dyn Extractor,
    work: &CountedWork,
) -> [u8; 32] {
    page_signature_of(
        work.parse_html(Row::PageParses, document.html),
        document,
        extractor,
        work,
    )
}

/// [`page_signature`] of `document`, whose untouched tree `tree` its caller
/// parsed once and the readers of the page as it was served read first:
/// the main-content step consumes it, so the page is parsed no more
/// (ADR 0138).
pub fn page_signature_of(
    tree: ParsedHtml,
    document: &Document<'_>,
    extractor: &dyn Extractor,
    work: &CountedWork,
) -> [u8; 32] {
    signed(title_and_main_text_of(tree, document.url, extractor, work))
}

/// The SHA-256 of `title`, a NUL and `text`.
fn signed((title, text): (String, String)) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(title.as_bytes());
    digest.update([0]);
    digest.update(text.as_bytes());
    digest.finalize().into()
}

#[cfg(test)]
mod tests;
