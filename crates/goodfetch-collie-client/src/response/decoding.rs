//! A response body's text and JSON (ADR 0119). A navigation's document
//! decodes with the encoding its document was decoded with, the browser's
//! `document.characterSet`; any other response with its BOM, its
//! `Content-Type` charset, the Encoding it declares in its own bytes (for
//! HTML Chrome's `<meta>` prescan then an XML declaration at the very
//! start, for an XML type that declaration, as `goodfetch_encoding` reads
//! it, the fetch tiers' own reading), and UTF-8 last. A byte that does not
//! decode becomes U+FFFD. JSON is UTF-8 whatever the charset, a leading
//! BOM ignored.

use encoding_rs::{Encoding, UTF_8};
use goodfetch_encoding::{Kind, content_type, declared};
use serde::de::DeserializeOwned;

use crate::error::Error;

/// `body` as text: decoded with `document`, the label of the encoding a
/// navigation's document was decoded with, when it names one; otherwise
/// with the encoding `body` and its `Content-Type` value `content_type`
/// declare. A BOM is left out, and a byte that does not decode becomes
/// U+FFFD.
pub(super) fn text(body: &[u8], document: Option<&str>, content_type: Option<&str>) -> String {
    let named = document.and_then(|label| Encoding::for_label(label.as_bytes()));
    let encoding = named.unwrap_or_else(|| of_body(body, content_type));
    let (text, _) = encoding.decode_with_bom_removal(body);
    text.into_owned()
}

/// `body`, the body of the response from `url`, as JSON: decoded as UTF-8
/// whatever its charset, a leading BOM ignored, a byte that does not
/// decode read as U+FFFD; `response_not_json` naming `url` when it is not
/// JSON, or not the JSON `T` reads.
pub(super) fn json<T: DeserializeOwned>(body: &[u8], url: String) -> Result<T, Error> {
    let (text, _) = UTF_8.decode_with_bom_removal(body);
    serde_json::from_str(&text).map_err(|source| Error::response_not_json(url, source))
}

/// The encoding `body`, whose `Content-Type` value is `value`, declares,
/// UTF-8 when none.
fn of_body(body: &[u8], value: Option<&str>) -> &'static Encoding {
    if let Some((encoding, _)) = Encoding::for_bom(body) {
        return encoding;
    }
    let (essence, charset) = value.map(content_type).unwrap_or_default();
    if let Some(encoding) = charset.and_then(|label| Encoding::for_label(label.as_bytes())) {
        return encoding;
    }
    declared(body, Kind::of(&essence)).unwrap_or(UTF_8)
}
