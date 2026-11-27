//! What a page's `Accept-Language` override hands the browser.

use super::languages;

/// The browser adds each language's weight itself, so an override given
/// the header's weights would send them twice (`en;q=0.9;q=0.9`).
#[test]
fn an_accept_language_header_is_handed_over_as_its_languages() {
    assert_eq!(languages("fr-FR,fr;q=0.9,en;q=0.8"), "fr-FR,fr,en");
    assert_eq!(languages(" en-US , en ; q=0.9"), "en-US,en");
    assert_eq!(languages("de"), "de");
}
