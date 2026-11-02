//! A page's signature (spec #581; ADR 0005): the SHA-256 of its document's
//! title and the text of its main content, each with its white space
//! collapsed, by which the Frontier tells whether two URLs serve one page.
//! Links, navigation and markup outside the main content's text take no
//! part, so two URLs of one page whose links carry their own parameters
//! still match.

use goodfetch_counted_work::CountedWork;
use sha2::{Digest as _, Sha256};

use crate::formats::Document;
use crate::main_content::title_and_main_text;

/// The page signature of `document`: the SHA-256 of its title, a NUL and
/// the text of its main content, which neither holds, so text moved from
/// one to the other makes another signature. A document with no main
/// content is signed by its title alone.
pub fn page_signature(document: &Document<'_>, work: &CountedWork) -> [u8; 32] {
    let (title, text) = title_and_main_text(document.html, document.url, work);
    let mut digest = Sha256::new();
    digest.update(title.as_bytes());
    digest.update([0]);
    digest.update(text.as_bytes());
    digest.finalize().into()
}

#[cfg(test)]
mod tests;
